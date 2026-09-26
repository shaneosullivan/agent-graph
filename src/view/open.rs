//! Opens a session in its agent, in a new terminal window, for the viewer's
//! Open button. The command comes from `resume::resume`, which only uses ids
//! that are plain letters, digits, `-` and `_`. The folder can hold anything,
//! so it never reaches a shell's parser unquoted: it's passed as the working
//! directory, as a separate argument, or quoted.
//!
//! - macOS: a one-off `.command` script, opened with `open`, so it runs in
//!   whichever terminal opens those (Terminal, unless you've changed it).
//! - Windows: `start` opens a new console window running `cmd /K`.
//! - Linux: `$TERMINAL`, or the first of several common terminals found.
//!
//! When the agent exits, the window stays open with a shell in the folder.
//!
//! The agent (and on Linux the terminal) is run by its full path, found on
//! `PATH` here, never by name in the session's folder: that folder can hold
//! anything, and Windows' `cmd` looks for commands in its current folder
//! first (as a relative `PATH` entry like `.` does elsewhere). For the same
//! reason the agent gets a `PATH` without relative entries, since it may
//! look programs up itself (`#!/usr/bin/env node`, npm's `.cmd` shims).

use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::resume::Resume;

/// Runs `r` in a new terminal window.
pub fn in_terminal(r: &Resume) -> Result<(), String> {
    let program =
        find_program(r.program).ok_or_else(|| format!("can't find {} on your PATH", r.program))?;
    let program = program.to_string_lossy();
    if cfg!(target_os = "macos") {
        macos(r, &program)
    } else if cfg!(windows) {
        windows(r, &program)
    } else {
        linux(r, &program)
    }
}

/// Where `program` is on `PATH`, skipping relative entries (like `.`), which
/// would make the answer depend on the current folder. On Windows, each of
/// `PATHEXT`'s extensions is tried (npm installs agents as `.cmd` files).
pub fn find_program(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let extensions = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into())
            .split(';')
            .filter(|e| !e.is_empty())
            .map(str::to_string)
            .collect()
    } else {
        vec![String::new()]
    };
    find_in(program, &path, &extensions)
}

/// `path` (a `PATH` value) without its relative entries.
fn absolute_only(path: &OsStr) -> OsString {
    let dirs: Vec<PathBuf> = std::env::split_paths(path)
        .filter(|dir| dir.is_absolute())
        .collect();
    std::env::join_paths(dirs).unwrap_or_default()
}

/// This process's `PATH`, for the agent: without relative entries. Never
/// empty, which would mean "look in the current folder" to many programs.
fn safe_path() -> OsString {
    let path = absolute_only(&std::env::var_os("PATH").unwrap_or_default());
    match (path.is_empty(), cfg!(windows)) {
        (true, false) => "/usr/bin:/bin".into(),
        _ => path,
    }
}

fn find_in(program: &str, path: &OsStr, extensions: &[String]) -> Option<PathBuf> {
    std::env::split_paths(path)
        .filter(|dir| dir.is_absolute())
        .flat_map(|dir| {
            extensions
                .iter()
                .map(move |ext| dir.join(format!("{program}{ext}")))
        })
        .find(|p| runnable(p))
}

fn runnable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.is_file() && meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        meta.is_file()
    }
}

/// The command to type yourself, for when a window can't be opened.
/// It names the agent by its full path when it's on `PATH`, so pasting it
/// doesn't run a program of the same name in the session's folder either.
pub fn command_line(r: &Resume) -> String {
    let program = find_program(r.program).map(|p| p.to_string_lossy().into_owned());
    command_line_for(r, program.as_deref(), cfg!(windows))
}

/// `command_line`, with `program` found (or not), for Windows' `cmd` or a
/// POSIX shell.
fn command_line_for(r: &Resume, program: Option<&str>, windows: bool) -> String {
    let args = r.args.join(" ");
    if windows {
        // In an interactive cmd, `&` and `(` inside quotes are just text.
        let program = program.map_or_else(|| r.program.to_string(), |p| format!("\"{p}\""));
        // Windows folders can't contain `"`; one in a crafted log would end
        // the quotes early, so it's dropped.
        format!(
            "set \"NoDefaultCurrentDirectoryInExePath=1\" && cd /d \"{}\" && {program} {args}",
            r.cwd.replace('"', "")
        )
    } else {
        let program = program.map_or_else(|| r.program.to_string(), sh_quote);
        format!("cd {} && {program} {args}", sh_quote(&r.cwd))
    }
}

fn macos(r: &Resume, program: &str) -> Result<(), String> {
    let id = ulid::Ulid::from_datetime(std::time::SystemTime::now());
    let path = std::env::temp_dir().join(format!("agent-graph-{id}.command"));
    write_script(&path, &macos_script(r, program))
        .map_err(|e| format!("writing {}: {e}", path.display()))?;
    let out = Command::new("open")
        .arg(&path)
        .output()
        .map_err(|e| format!("running open: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        let _ = std::fs::remove_file(&path);
        Err(format!(
            "open couldn't run it: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// Keeps only the absolute entries of `PATH`. (Terminal gives the script its
/// own environment, so it has to do this itself.)
const ABSOLUTE_PATH_ONLY: &str = "set -f; kept=; IFS=:; for d in $PATH; do case $d in /*) \
kept=\"$kept${kept:+:}$d\";; esac; done; unset IFS; set +f; PATH=${kept:-/usr/bin:/bin}; \
export PATH";

/// A script that deletes itself, runs the agent (`program`, its full path)
/// in the session's folder, with a `PATH` of absolute entries only, then
/// leaves a shell there.
fn macos_script(r: &Resume, program: &str) -> String {
    let command = std::iter::once(program)
        .chain(r.args.iter().map(String::as_str))
        .map(sh_quote)
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "#!/bin/sh\n# Written by agent-graph view to open a session. It deletes itself.\n\
         rm -f -- \"$0\"\n{ABSOLUTE_PATH_ONLY}\ncd -- {} || exit 1\n{command}\nexec \"${{SHELL:-/bin/sh}}\" -l\n",
        sh_quote(&r.cwd)
    )
}

fn write_script(path: &Path, text: &str) -> io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o700);
    }
    options.open(path)?.write_all(text.as_bytes())
}

/// Characters `cmd` treats specially. The agent's path goes through two of
/// its parses (`/C start …`, then `/K …`), where quoting doesn't reliably
/// protect them, so a path with any of these isn't used.
const CMD_SPECIAL: &[char] = &['&', '<', '>', '(', ')', '@', '^', '|', '%', '!', '"'];

/// Whether `path` can go on a `cmd` command line as it is.
fn cmd_safe(path: &str) -> bool {
    !path.contains(CMD_SPECIAL)
}

fn windows(r: &Resume, program: &str) -> Result<(), String> {
    if !cmd_safe(program) {
        return Err(format!(
            "{program} has characters Windows' cmd can't be trusted to pass along"
        ));
    }
    // %ComSpec% (cmd.exe's full path) rather than a PATH search.
    let cmd = std::env::var_os("ComSpec").unwrap_or_else(|| "cmd".into());
    let status = Command::new(cmd)
        .args(windows_args(r, program))
        .current_dir(&r.cwd)
        // As well as the full path: don't look in the current folder for
        // anything run by name, or in any relative PATH entry.
        .env("NoDefaultCurrentDirectoryInExePath", "1")
        .env("PATH", safe_path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("running cmd: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("start failed ({status})"))
    }
}

/// `cmd /C start "Agent Graph" cmd /K <command>`: a new window titled
/// "Agent Graph" that stays open after the agent exits. The folder isn't in
/// here: `start` runs it in the current directory, which is set to it.
fn windows_args(r: &Resume, program: &str) -> Vec<String> {
    ["/C", "start", "Agent Graph", "cmd", "/K", program]
        .into_iter()
        .map(String::from)
        .chain(r.args.iter().cloned())
        .collect()
}

/// Terminals to look for, and what goes before the command to run in them.
const TERMINALS: &[(&str, &[&str])] = &[
    ("gnome-terminal", &["--"]),
    ("konsole", &["-e"]),
    ("xfce4-terminal", &["-x"]),
    ("kitty", &[]),
    ("alacritty", &["-e"]),
    ("wezterm", &["start", "--"]),
    ("foot", &[]),
    ("x-terminal-emulator", &["-e"]),
    ("xterm", &["-e"]),
];

fn linux(r: &Resume, agent: &str) -> Result<(), String> {
    if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return Err("there's no desktop here to open a terminal on".to_string());
    }
    let chosen = std::env::var("TERMINAL").ok().filter(|t| !t.is_empty());
    let (program, before): (String, &[&str]) = match chosen {
        Some(t) => (t, &["-e"]),
        None => TERMINALS
            .iter()
            .find_map(|(name, before)| {
                find_program(name).map(|p| (p.to_string_lossy().into_owned(), *before))
            })
            .ok_or("couldn't find a terminal; set $TERMINAL to yours")?,
    };
    let mut child = Command::new(&program)
        .args(linux_args(before, r, agent))
        .env("PATH", safe_path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("running {program}: {e}"))?;
    // Some terminals stay in the foreground until the window closes: reap
    // them then, rather than leave a zombie behind.
    std::thread::spawn(move || child.wait());
    Ok(())
}

/// `<before> /bin/sh -c <script> agent-graph <folder> <agent> <args…>`. The
/// script gets the folder and command as arguments, so neither is ever
/// parsed as shell.
fn linux_args(before: &[&str], r: &Resume, agent: &str) -> Vec<String> {
    // The PATH filter again here: a terminal that hands the command to an
    // instance already running may not pass our environment on.
    let script =
        format!(r#"{ABSOLUTE_PATH_ONLY}; cd -- "$1" && shift && "$@"; exec "${{SHELL:-/bin/sh}}""#);
    before
        .iter()
        .copied()
        .chain(["/bin/sh", "-c", &script, "agent-graph", &r.cwd, agent])
        .map(String::from)
        .chain(r.args.iter().cloned())
        .collect()
}

/// Quotes `s` as one word for a POSIX shell.
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resume(cwd: &str) -> Resume {
        Resume {
            app: "Claude Code",
            program: "claude",
            args: vec!["--resume".into(), "abc-123".into()],
            cwd: cwd.into(),
            copy: false,
        }
    }

    #[test]
    fn the_macos_script_quotes_the_folder() {
        let script = macos_script(&resume("/w/it's $HOME `x`"), "/opt/bin/claude");
        assert!(script.starts_with("#!/bin/sh\n"));
        assert!(script.contains("\ncd -- '/w/it'\\''s $HOME `x`' || exit 1\n"));
        assert!(script.contains("\n'/opt/bin/claude' '--resume' 'abc-123'\n"));
        assert!(script.contains("rm -f -- \"$0\""));
    }

    #[test]
    fn linux_passes_the_folder_as_an_argument() {
        let args = linux_args(&["--"], &resume("/w/a b;c"), "/opt/bin/claude");
        assert_eq!(
            args,
            [
                "--",
                "/bin/sh",
                "-c",
                &format!(
                    r#"{ABSOLUTE_PATH_ONLY}; cd -- "$1" && shift && "$@"; exec "${{SHELL:-/bin/sh}}""#
                ),
                "agent-graph",
                "/w/a b;c",
                "/opt/bin/claude",
                "--resume",
                "abc-123",
            ]
        );
    }

    #[test]
    fn windows_keeps_the_folder_off_the_command_line() {
        let args = windows_args(&resume(r"C:\w\a&b"), r"C:\npm\claude.cmd");
        assert_eq!(
            args,
            [
                "/C",
                "start",
                "Agent Graph",
                "cmd",
                "/K",
                r"C:\npm\claude.cmd",
                "--resume",
                "abc-123"
            ]
        );
    }

    /// R8: the agent is found on PATH by full path; a relative entry (like
    /// `.`, the session's folder when it runs) is never used.
    #[test]
    fn the_agent_is_found_by_full_path_and_never_in_the_folder() {
        let root = tempfile::tempdir().unwrap();
        let session = root.path().join("session");
        let bin = root.path().join("bin");
        std::fs::create_dir_all(&session).unwrap();
        std::fs::create_dir_all(&bin).unwrap();
        let name = if cfg!(windows) {
            "claude.cmd"
        } else {
            "claude"
        };
        for dir in [&session, &bin] {
            let file = dir.join(name);
            std::fs::write(&file, "").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
        let exts: Vec<String> = if cfg!(windows) {
            vec![".exe".into(), ".cmd".into()]
        } else {
            vec![String::new()]
        };
        let path = |dirs: &[&Path]| std::env::join_paths(dirs).unwrap();

        // A relative entry that really does lead to a runnable `claude` (a
        // folder in the current one, like `.` would be the session's folder
        // when the agent runs) is skipped.
        let here = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let planted = here.path().join(name);
        std::fs::write(&planted, "").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&planted, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let relative = PathBuf::from(here.path().file_name().unwrap());
        assert!(relative.join(name).is_file(), "reachable relative to here");
        let found = find_in("claude", &path(&[&relative, &bin]), &exts);
        assert_eq!(found, Some(bin.join(name)));
        assert_eq!(find_in("claude", &path(&[&relative]), &exts), None);
        assert_eq!(
            find_in("claude", &path(&[&session, &bin]), &exts),
            Some(session.join(name)),
            "absolute entries are used, in order"
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let plain = root.path().join("plain");
            std::fs::create_dir_all(&plain).unwrap();
            std::fs::write(plain.join("claude"), "").unwrap();
            std::fs::set_permissions(plain.join("claude"), std::fs::Permissions::from_mode(0o644))
                .unwrap();
            assert_eq!(
                find_in("claude", &path(&[&plain, &bin]), &exts),
                Some(bin.join("claude")),
                "a file that can't be run isn't the program"
            );
        }
    }

    /// R8: the agent's PATH keeps only absolute entries: here, and in the
    /// macOS script, run by a real shell.
    #[test]
    fn relative_path_entries_are_dropped_for_the_agent() {
        let sep = if cfg!(windows) { ";" } else { ":" };
        let abs = |p: &str| {
            if cfg!(windows) {
                format!("C:\\{p}")
            } else {
                format!("/{p}")
            }
        };
        let given = [
            abs("usr"),
            ".".into(),
            "node_modules/.bin".into(),
            abs("bin"),
        ]
        .join(sep);
        let kept = absolute_only(OsStr::new(&given));
        assert_eq!(kept.to_string_lossy(), [abs("usr"), abs("bin")].join(sep));

        #[cfg(unix)]
        {
            let out = Command::new("/bin/sh")
                .args(["-c", &format!("{ABSOLUTE_PATH_ONLY}; printf %s \"$PATH\"")])
                .env("PATH", "/usr/bin:.:node_modules/.bin:/b*n:/bin")
                .output()
                .unwrap();
            assert_eq!(String::from_utf8_lossy(&out.stdout), "/usr/bin:/b*n:/bin");
            let only_relative = Command::new("/bin/sh")
                .args(["-c", &format!("{ABSOLUTE_PATH_ONLY}; printf %s \"$PATH\"")])
                .env("PATH", ".:rel")
                .output()
                .unwrap();
            assert_eq!(
                String::from_utf8_lossy(&only_relative.stdout),
                "/usr/bin:/bin",
                "never empty, which would mean the current folder"
            );
            assert!(macos_script(&resume("/w"), "/opt/claude").contains(ABSOLUTE_PATH_ONLY));
        }
    }

    /// R8: on Windows, a path with characters cmd treats specially isn't
    /// passed through it (it'd be split, or lose its quotes).
    #[test]
    fn paths_cmd_would_misread_are_refused() {
        assert!(cmd_safe(r"C:\Program Files\nodejs\claude.cmd"));
        for bad in [
            r"C:\Users\R&D\AppData\Roaming\npm\claude.cmd",
            r"C:\Users\John Smith (Work)\npm\claude.cmd",
            r"C:\Users\a^b\claude.cmd",
            r"C:\Users\100%\claude.cmd",
        ] {
            assert!(!cmd_safe(bad), "{bad}");
        }
    }

    #[test]
    fn the_command_to_type_yourself() {
        // R8: by full path when it's found, so pasting it can't run a
        // program planted in the session's folder.
        assert_eq!(
            command_line_for(&resume("/w/my app"), Some("/opt/bin/claude"), false),
            "cd '/w/my app' && '/opt/bin/claude' --resume abc-123"
        );
        assert_eq!(
            command_line_for(
                &resume(r"C:\w\app"),
                Some(r"C:\Users\R&D\npm\claude.cmd"),
                true
            ),
            r#"set "NoDefaultCurrentDirectoryInExePath=1" && cd /d "C:\w\app" && "C:\Users\R&D\npm\claude.cmd" --resume abc-123"#
        );
        assert!(
            command_line_for(&resume(r#"C:\w" & calc & ""#), None, true)
                .contains(r#"cd /d "C:\w & calc & " && claude"#)
        );
        // Not found: all it can do is name it.
        assert_eq!(
            command_line_for(&resume("/w/app"), None, false),
            "cd '/w/app' && claude --resume abc-123"
        );
    }
}
