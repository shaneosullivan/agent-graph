//! `agent-graph run -- <command>`: runs a command as a node in the graph,
//! linked to whatever started it, with anything it starts linked to it.
//!
//! It brings in tools without hooks (`agent-graph run -- aider`), and groups
//! the sessions a script starts (`agent-graph run -- ./workers.sh`). The
//! command gets `AGENT_GRAPH_PARENT` and `TRACEPARENT`, so agents it starts
//! that do have hooks link themselves under it (design §6, method 2). Its node
//! is working until the command exits, then completed, or failed with the
//! exit code. Its input and output pass straight through.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::{Command, ExitCode, ExitStatus};
use std::time::SystemTime;

use crate::adapter::Draft;
use crate::event::{Payload, SessionEnded, SessionStarted, Source, State, Status};
use crate::{emit, link, paths, process, store};

/// The provider name of the nodes `run` makes: `run:<id>`.
pub const PROVIDER: &str = "run";

pub struct Options {
    /// What to call the node. Defaults to the program's name.
    pub name: Option<String>,
    /// The command and its arguments.
    pub command: Vec<OsString>,
}

pub fn run(opts: Options) -> ExitCode {
    let Some(program) = opts.command.first() else {
        eprintln!("agent-graph run: give a command to run, after --");
        return ExitCode::from(2);
    };
    let id = ulid::Ulid::from_datetime(SystemTime::now()).to_string();
    let node = format!("{PROVIDER}:{id}");
    let mut log = Log::new(&id);

    let var = |name: &str| std::env::var(name).ok();
    let parent = link::parent_from(var(link::PARENT_VAR).as_deref(), &node);
    let traceparent = link::traceparent(&node, var(link::TRACEPARENT_VAR).as_deref());
    let lineage = process::lineage(std::process::id());
    let title = opts
        .name
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| program_name(program));

    let mut start = Draft::new(
        &node,
        Payload::SessionStarted(SessionStarted {
            cwd: std::env::current_dir()
                .ok()
                .map(|d| d.display().to_string()),
            source: Some("run".to_string()),
            title: Some(crate::event::truncate_chars(&title, 200)),
            transcript_path: None,
            link_method: parent.as_deref().map(|p| link::method_for(p).to_string()),
            process: lineage.first().map(process::Process::id),
            ancestors: lineage.iter().skip(1).map(process::Process::id).collect(),
        }),
    );
    start.parent = parent;
    start.trace = Some(serde_json::json!({ "traceparent": traceparent }));
    log.record(vec![start, status(&node, State::Working, None)]);

    let mut command = Command::new(program_path(program));
    command
        .args(&opts.command[1..])
        .env(link::PARENT_VAR, &node)
        .env(link::TRACEPARENT_VAR, &traceparent);
    signals::stay_for_the_child();
    let outcome = command.spawn().and_then(|mut child| {
        signals::forward_to(child.id());
        child.wait()
    });

    let (code, end) = match outcome {
        Ok(status) => (exit_code(status), ending(status)),
        Err(e) => {
            eprintln!(
                "agent-graph run: can't run {}: {e}",
                program.to_string_lossy()
            );
            let code = if e.kind() == std::io::ErrorKind::NotFound {
                127
            } else {
                126
            };
            (code, Some((State::Failed, format!("Couldn't start: {e}"))))
        }
    };
    let mut drafts = Vec::new();
    if let Some((state, summary)) = end {
        drafts.push(status(&node, state, Some(summary)));
    }
    drafts.push(Draft::new(
        &node,
        Payload::SessionEnded(SessionEnded {
            reason: Some(format!("exit {code}")),
        }),
    ));
    log.record(drafts);
    match exit_with(code, cfg!(windows)) {
        Exit::Code(code) => ExitCode::from(code),
        Exit::Raw(code) => std::process::exit(code),
    }
}

#[derive(Debug, PartialEq)]
enum Exit {
    /// A code `ExitCode` can hold.
    Code(u8),
    /// One it can't, passed to the system as it is.
    Raw(i32),
}

/// How to exit with the command's `code`. On Windows it's 32 bits (a crash
/// is 0xC0000005, say), which only `process::exit` passes on whole.
fn exit_with(code: i32, windows: bool) -> Exit {
    match u8::try_from(code) {
        Ok(code) => Exit::Code(code),
        Err(_) if windows => Exit::Raw(code),
        Err(_) => Exit::Code(1),
    }
}

/// Where the node's events go. Recording is best effort: the command runs
/// even if they can't be written.
struct Log {
    file: Option<PathBuf>,
    source: Source,
    warned: bool,
}

impl Log {
    fn new(id: &str) -> Log {
        let file = paths::data_dir().map(|root| {
            paths::events_dir(&root).join(format!("{}.jsonl", paths::file_key(PROVIDER, id)))
        });
        Log {
            file,
            source: Source {
                provider: PROVIDER.to_string(),
                provider_version: None,
                adapter: None,
            },
            warned: false,
        }
    }

    fn record(&mut self, drafts: Vec<Draft>) {
        let text: String = emit::stamp(drafts, &self.source, SystemTime::now())
            .iter()
            .map(|e| emit::to_line(e) + "\n")
            .collect();
        let result = match &self.file {
            Some(file) => file
                .parent()
                .map_or(Ok(()), store::ensure_dir)
                .and_then(|()| store::append(file, text.as_bytes()))
                .map_err(|e| format!("{}: {e}", file.display())),
            None => Err("can't find your home directory".to_string()),
        };
        if let Err(e) = result {
            if !self.warned {
                eprintln!("agent-graph run: not recording this run: {e}");
                self.warned = true;
            }
        }
    }
}

fn status(node: &str, state: State, summary: Option<String>) -> Draft {
    Draft::new(node, Payload::Status(Status { state, summary }))
}

/// How the node ends, beyond `session.ended`: nothing for a clean exit.
fn ending(status: ExitStatus) -> Option<(State, String)> {
    if status.success() {
        return None;
    }
    Some(match (status.code(), signal(status)) {
        (Some(code), _) => (State::Failed, format!("Exited with code {code}")),
        (None, Some(sig)) => (State::Canceled, format!("Stopped by signal {sig}")),
        (None, None) => (State::Failed, "Exited abnormally".to_string()),
    })
}

/// The code to exit with: the command's, or 128 + the signal that stopped it,
/// as shells report it.
fn exit_code(status: ExitStatus) -> i32 {
    status
        .code()
        .or_else(|| signal(status).map(|s| 128 + s))
        .unwrap_or(1)
}

#[cfg(unix)]
fn signal(status: ExitStatus) -> Option<i32> {
    std::os::unix::process::ExitStatusExt::signal(&status)
}

#[cfg(not(unix))]
fn signal(_status: ExitStatus) -> Option<i32> {
    None
}

/// What to start for `program`. On Windows, `Command` only finds `.exe`
/// files by name, but npm installs agents (`codex`, `gemini`, often
/// `claude`) as `.cmd` files, so a bare name is looked up through `PATH` and
/// `PATHEXT` as a shell would. Elsewhere, and for a path or a name with an
/// extension, it's left to `Command`.
fn program_path(program: &OsString) -> OsString {
    if cfg!(windows) {
        let name = program.to_string_lossy();
        let bare = !name.contains(['/', '\\', '.']);
        if bare {
            if let Some(found) = crate::paths::find_program(&name) {
                return found.into_os_string();
            }
        }
    }
    program.clone()
}

/// A program's name without its folder or Windows extension: as the shell
/// adapter names the program in the request that started this, so a
/// session can be paired with it (`reducer::runs`).
fn program_name(program: &OsString) -> String {
    crate::adapter::shell::program_name(&program.to_string_lossy())
}

/// Ctrl+C in a terminal goes to the command and to us. We stay until the
/// command exits, so its end is recorded: handlers that do nothing (unlike
/// ignoring the signal, those aren't inherited by the command). A hang-up or
/// `kill` sent to us alone is passed on to the command.
#[cfg(unix)]
mod signals {
    use std::sync::atomic::{AtomicI32, Ordering};

    static CHILD: AtomicI32 = AtomicI32::new(0);

    extern "C" fn stay(_: libc::c_int) {}

    extern "C" fn pass_on(sig: libc::c_int) {
        let child = CHILD.load(Ordering::SeqCst);
        if child > 0 {
            // SAFETY: kill is async-signal-safe.
            unsafe { libc::kill(child, sig) };
        }
    }

    /// A signal we were started with ignored (by `nohup`, or as a
    /// background job) is left ignored, so the command inherits that too:
    /// a handler would be reset to the default for it.
    pub fn stay_for_the_child() {
        let handlers: [(libc::c_int, extern "C" fn(libc::c_int)); 4] = [
            (libc::SIGINT, stay),
            (libc::SIGQUIT, stay),
            (libc::SIGHUP, pass_on),
            (libc::SIGTERM, pass_on),
        ];
        for (sig, handler) in handlers {
            if !ignored(sig) {
                // SAFETY: both handlers are async-signal-safe.
                unsafe { libc::signal(sig, handler as *const () as libc::sighandler_t) };
            }
        }
    }

    /// Whether `sig` is set to be ignored.
    fn ignored(sig: libc::c_int) -> bool {
        // SAFETY: a null new action only reads the current one into `old`.
        unsafe {
            let mut old: libc::sigaction = std::mem::zeroed();
            libc::sigaction(sig, std::ptr::null(), &mut old) == 0
                && old.sa_sigaction == libc::SIG_IGN
        }
    }

    pub fn forward_to(pid: u32) {
        CHILD.store(i32::try_from(pid).unwrap_or(0), Ordering::SeqCst);
    }
}

#[cfg(windows)]
mod signals {
    use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;
    use windows_sys::core::BOOL;

    /// Ctrl+C and Ctrl+Break: handled (by waiting for the command). Closing
    /// the window, logging off or shutting down: the default.
    unsafe extern "system" fn stay(kind: u32) -> BOOL {
        BOOL::from(kind <= 1)
    }

    pub fn stay_for_the_child() {
        // SAFETY: registers a handler that only returns a value.
        unsafe { SetConsoleCtrlHandler(Some(stay), 1) };
    }

    pub fn forward_to(_pid: u32) {}
}

#[cfg(not(any(unix, windows)))]
mod signals {
    pub fn stay_for_the_child() {}
    pub fn forward_to(_pid: u32) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    /// R50: Windows exit codes are 32 bits (a crash is 0xC0000005, say),
    /// and `run` exits with the command's, whatever it is.
    #[test]
    fn windows_exit_codes_pass_through_whole() {
        assert_eq!(exit_with(0, true), Exit::Code(0));
        assert_eq!(exit_with(255, true), Exit::Code(255));
        assert_eq!(exit_with(256, true), Exit::Raw(256));
        assert_eq!(
            exit_with(0xC000_0005_u32 as i32, true),
            Exit::Raw(-1073741819)
        );
        // Elsewhere codes fit in a byte (a signal is 128 + its number).
        assert_eq!(exit_with(130, false), Exit::Code(130));
    }

    #[test]
    fn names_come_from_the_program() {
        assert_eq!(program_name(&"aider".into()), "aider");
        assert_eq!(program_name(&"/usr/bin/codex".into()), "codex");
        assert_eq!(program_name(&"./workers.sh".into()), "workers.sh");
        // As the request that started it names it (R25).
        for word in ["worker.cmd", "Worker.EXE", "tools/run.bat", "./My Agent"] {
            let launch =
                crate::adapter::shell::agent_launch(&format!("agent-graph run -- '{word}'"), &[]);
            assert_eq!(
                launch.map(|l| l.program),
                Some(program_name(&word.into())),
                "{word}"
            );
        }
        if cfg!(windows) {
            assert_eq!(program_name(&r"C:\bin\claude.exe".into()), "claude");
        }
    }
}
