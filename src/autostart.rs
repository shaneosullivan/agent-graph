//! `agent-graph watch-remote --autostart`: share live whenever you log in to
//! this computer, without running anything yourself.
//!
//! It's the system's own "run this at login, and keep it running":
//!
//! - macOS: a launchd agent, `~/Library/LaunchAgents/<LABEL>.plist`, loaded
//!   into the login session (`launchctl bootstrap gui/<uid>`). Its output
//!   goes to `watch-remote.log` in the data directory.
//! - Linux: a systemd user service,
//!   `~/.config/systemd/user/<UNIT>` (or under `$XDG_CONFIG_HOME`), enabled
//!   and started (`systemctl --user enable --now`). Its output goes to the
//!   journal (`journalctl --user -u <UNIT>`).
//! - Windows: an entry in the user's Run key (`RUN_KEY`, named
//!   `RUN_VALUE`), which Windows runs at login, and lists in Task Manager's
//!   Startup apps. Windows has no service of its own that runs a console
//!   program without a window and needs no administrator, so the entry runs
//!   `watch-remote --autostarted`, which starts a copy of itself without a
//!   window (its window only flashes) to do what launchd and systemd do
//!   elsewhere: run `watch-remote --background`, again a minute after it
//!   fails (`autostarted`). Its output goes to `watch-remote.log` in the
//!   data directory, and its process is in `watch-remote.pid` there, to
//!   stop it.
//!
//! Each runs `agent-graph watch-remote --background` by the path that
//! outlasts upgrades (`install::lasting_exe`), and starts it again a minute
//! after it fails (the network not being up yet at login, say). A
//! background run never opens a browser: one that needs the user (to log in
//! again, say) says so in its log and stops without failing, so it isn't
//! started again and again (see `remote::run`).
//!
//! It's set up for someone logged in: whoever isn't logs in first, in the
//! browser, as `watch-remote` does, since a background run can't.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The launchd agent's label, and its file's name.
pub const LABEL: &str = "com.chofter.agent-graph.watch-remote";
/// The systemd user service's name.
pub const UNIT: &str = "agent-graph-watch-remote.service";
/// How long after failing it's started again, in seconds.
const RESTART_AFTER: u32 = 60;
/// The user's Run key on Windows, under HKEY_CURRENT_USER.
pub const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
/// The Run key's entry for it.
pub const RUN_VALUE: &str = "AgentGraphWatchRemote";
/// Set for the copy `--autostarted` starts without a window, which keeps
/// `watch-remote` running (Windows).
const SUPERVISOR_VAR: &str = "AGENT_GRAPH_AUTOSTART_SUPERVISOR";

/// The arguments the service runs `agent-graph` with, for the site `url`.
pub fn args(url: &str) -> Vec<String> {
    let url = url.trim_end_matches('/');
    let mut args = vec!["watch-remote".to_string(), "--background".to_string()];
    if url != crate::remote::DEFAULT_URL {
        args.push(format!("--url={url}"));
    }
    args
}

/// Whether it can be started at login on this system.
pub fn supported() -> bool {
    cfg!(any(target_os = "macos", target_os = "linux", windows))
}

/// What the Run key's entry runs at login (Windows): this `exe`,
/// `--autostarted`, for the site `url`, with `home` as the data directory
/// if it's set (the entry can't set AGENT_GRAPH_HOME).
pub fn login_command(exe: &Path, url: &str, home: Option<&Path>) -> String {
    let url = url.trim_end_matches('/');
    let mut words = vec![
        exe.to_string_lossy().into_owned(),
        "watch-remote".to_string(),
        "--autostarted".to_string(),
    ];
    if url != crate::remote::DEFAULT_URL {
        words.push(format!("--url={url}"));
    }
    if let Some(home) = home {
        words.push(format!("--data-dir={}", home.to_string_lossy()));
    }
    words
        .iter()
        .map(|w| windows_arg(w))
        .collect::<Vec<_>>()
        .join(" ")
}

/// One word of a Windows command line, as programs split them
/// (`CommandLineToArgvW`, and the C runtime): in double quotes, with a
/// quote escaped by a backslash, and the backslashes before a quote (or the
/// closing one) doubled.
fn windows_arg(s: &str) -> String {
    let mut out = String::from("\"");
    let mut backslashes = 0;
    for c in s.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.push_str(&"\\".repeat(backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.push_str(&"\\".repeat(backslashes));
                backslashes = 0;
                out.push(c);
            }
        }
    }
    out.push_str(&"\\".repeat(backslashes * 2));
    out.push('"');
    out
}

/// Where the service's file goes, on macOS or Linux; an error elsewhere.
pub fn service_path() -> Result<PathBuf, String> {
    let home = crate::paths::user_home().ok_or("can't find your home directory")?;
    if cfg!(target_os = "macos") {
        Ok(home
            .join("Library/LaunchAgents")
            .join(format!("{LABEL}.plist")))
    } else if cfg!(target_os = "linux") {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"));
        Ok(config.join("systemd/user").join(UNIT))
    } else {
        Err("starting it when you log in isn't supported on this system yet".into())
    }
}

/// The launchd agent: runs `exe` with `args` at login, and again
/// `RESTART_AFTER` seconds after it fails (not after it stops by itself),
/// its output appended to `log`, with `home` as `AGENT_GRAPH_HOME` if it's
/// set.
pub fn launchd_plist(exe: &Path, args: &[String], log: &Path, home: Option<&Path>) -> String {
    let string = |s: &str| format!("<string>{}</string>", xml_escape(s));
    let mut program = vec![string(&exe.to_string_lossy())];
    program.extend(args.iter().map(|a| string(a)));
    let env = home.map_or(String::new(), |home| {
        format!(
            "  <key>EnvironmentVariables</key>\n  <dict>\n    <key>AGENT_GRAPH_HOME</key>\n    {}\n  </dict>\n",
            string(&home.to_string_lossy())
        )
    });
    let log = string(&log.to_string_lossy());
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<!-- Written by `agent-graph watch-remote --autostart`; removed by --no-autostart. -->
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    {program}
  </array>
{env}  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <dict>
    <key>SuccessfulExit</key>
    <false/>
  </dict>
  <key>ThrottleInterval</key>
  <integer>{RESTART_AFTER}</integer>
  <key>ProcessType</key>
  <string>Background</string>
  <key>StandardOutPath</key>
  {log}
  <key>StandardErrorPath</key>
  {log}
</dict>
</plist>
"#,
        program = program.join("\n    "),
    )
}

/// The systemd user service: runs `exe` with `args` once the user's logged
/// in, and again `RESTART_AFTER` seconds after it fails, with `home` as
/// `AGENT_GRAPH_HOME` if it's set.
pub fn systemd_unit(exe: &Path, args: &[String], home: Option<&Path>) -> String {
    let word = |s: &str| systemd_quote(&s.replace('$', "$$"));
    let mut exec = vec![word(&exe.to_string_lossy())];
    exec.extend(args.iter().map(|a| word(a)));
    let env = home.map_or(String::new(), |home| {
        format!(
            "Environment={}\n",
            systemd_quote(&format!("AGENT_GRAPH_HOME={}", home.to_string_lossy()))
        )
    });
    format!(
        "# Written by `agent-graph watch-remote --autostart`; removed by --no-autostart.\n\
         [Unit]\n\
         Description=Agent Graph: share your agents' sessions live, to your account\n\
         Wants=network-online.target\n\
         After=network-online.target\n\
         \n\
         [Service]\n\
         ExecStart={exec}\n\
         {env}\
         Restart=on-failure\n\
         RestartSec={RESTART_AFTER}\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        exec = exec.join(" "),
    )
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// A systemd setting's value, or (with `$` doubled too, which only a command
/// line expands) one word of a command line: double-quoted, with
/// backslashes and quotes escaped, and `%` (a specifier) doubled.
fn systemd_quote(s: &str) -> String {
    let inner = s
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%");
    format!("\"{inner}\"")
}

/// Sets it up (again, if it was: with this executable and `url`), and
/// starts it, logging in to `url` first if this computer isn't (see the
/// module docs).
pub fn enable(root: &Path, url: &str) -> Result<(), String> {
    let url = url.trim_end_matches('/');
    // (Checked before logging in: no use logging in where it can't be set up.)
    if !supported() {
        return Err("starting it when you log in isn't supported on this system yet".into());
    }
    if !crate::remote::sends_privately(url) {
        return Err(format!(
            "{url} isn't HTTPS, so your login would be sent in the clear. Use an https:// URL."
        ));
    }
    let client = crate::remote::Client::new(url);
    // (An API token given as AGENT_GRAPH_TOKEN is saved as the login, so the
    // service has it: see `account::from_env`.)
    let account = match crate::account::from_env(root, url, &client)
        .map_err(crate::account::TokenError::message)?
        .or_else(|| crate::account::load(root, url))
    {
        Some(account) => account,
        None => crate::account::login(root, url, &client)?,
    };
    let exe = crate::install::lasting_exe()?;
    let args = args(url);
    let home = std::env::var_os("AGENT_GRAPH_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    let log = root.join("watch-remote.log");
    if cfg!(windows) {
        win::set_run_entry(&login_command(&exe, url, home.as_deref()))?;
        if manages_services() {
            // (Running already, from before: stopped first, so this one's used.)
            stop_running(root);
            win::start_supervisor(
                &std::env::current_exe().map_err(|e| e.to_string())?,
                root,
                url,
            )?;
        }
        println!(
            "It'll share your sessions live to {url}/watch whenever you log in to this computer, as {}. It's started now too.",
            account.who()
        );
        println!(
            "It's in Task Manager's Startup apps. What it says goes to {}.",
            log.display()
        );
        println!("To stop it, and not start it again: agent-graph watch-remote --no-autostart");
        return Ok(());
    }
    let contents = if cfg!(target_os = "macos") {
        launchd_plist(&exe, &args, &log, home.as_deref())
    } else {
        systemd_unit(&exe, &args, home.as_deref())
    };
    let path = service_path()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    }
    std::fs::write(&path, contents).map_err(|e| format!("writing {}: {e}", path.display()))?;

    if manages_services() {
        if cfg!(target_os = "macos") {
            let domain = gui_domain()?;
            // (Loaded already, from before: unloaded first, so the new file's used.)
            let _ = run("launchctl", &["bootout", &format!("{domain}/{LABEL}")]);
            run(
                "launchctl",
                &["bootstrap", &domain, &path.to_string_lossy()],
            )?;
        } else {
            run("systemctl", &["--user", "daemon-reload"])?;
            run("systemctl", &["--user", "enable", UNIT])?;
            run("systemctl", &["--user", "restart", UNIT])?;
        }
    }
    println!(
        "It'll share your sessions live to {url}/watch whenever you log in to this computer, as {}. It's started now too.",
        account.who()
    );
    if cfg!(target_os = "macos") {
        println!("What it says goes to {}.", log.display());
    } else {
        println!("What it says goes to the journal: journalctl --user -u {UNIT}");
    }
    println!("To stop it, and not start it again: agent-graph watch-remote --no-autostart");
    Ok(())
}

/// Stops it, and removes it, so it isn't started at login any more.
pub fn disable(root: &Path) -> Result<(), String> {
    if cfg!(windows) {
        let there = win::run_entry().is_some();
        if manages_services() {
            stop_running(root);
        }
        if there {
            win::remove_run_entry()?;
            println!("Stopped. It won't start when you log in any more.");
        } else {
            println!("It wasn't set to start when you log in.");
        }
        return Ok(());
    }
    let path = service_path()?;
    let there = path.exists();
    if manages_services() {
        if cfg!(target_os = "macos") {
            let _ = run(
                "launchctl",
                &["bootout", &format!("{}/{LABEL}", gui_domain()?)],
            );
        } else {
            let _ = run("systemctl", &["--user", "disable", "--now", UNIT]);
        }
    }
    if there {
        std::fs::remove_file(&path).map_err(|e| format!("removing {}: {e}", path.display()))?;
        if manages_services() && cfg!(target_os = "linux") {
            let _ = run("systemctl", &["--user", "daemon-reload"]);
        }
        println!("Stopped. It won't start when you log in any more.");
    } else {
        println!("It wasn't set to start when you log in.");
    }
    Ok(())
}

/// Whether it's set up to start at login.
pub fn enabled() -> bool {
    if cfg!(windows) {
        return win::run_entry().is_some();
    }
    service_path().is_ok_and(|p| p.exists())
}

/// `watch-remote --autostarted`, which the Run key's entry runs at login on
/// Windows (elsewhere it's a background run). Started with a window, it
/// starts a copy of itself without one, and stops; that copy (the
/// supervisor: `SUPERVISOR_VAR` is set) runs `watch-remote --background`,
/// its output appended to the log, and again `RESTART_AFTER` seconds after
/// it fails, until it stops by itself. Its process is in the pid file
/// meanwhile, so `--no-autostart` (or setting it up again) can stop it.
pub fn autostarted(root: &Path, url: &str) -> Result<(), String> {
    if !cfg!(windows) {
        return crate::remote::run(
            root,
            crate::remote::Options {
                url: url.to_string(),
                session: None,
                new: false,
                logout: false,
                background: true,
            },
        );
    }
    let exe = std::env::current_exe().map_err(|e| format!("can't find this executable: {e}"))?;
    if std::env::var_os(SUPERVISOR_VAR).is_none() {
        return win::start_supervisor(&exe, root, url);
    }
    let pid_file = pid_file(root);
    let me = crate::process::lineage(std::process::id())
        .first()
        .map(crate::process::Process::id);
    if let Some(me) = &me {
        let _ = std::fs::write(&pid_file, me);
    }
    let log = root.join("watch-remote.log");
    let note = |what: &str| {
        use std::io::Write;
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log)
        {
            let _ = writeln!(
                file,
                "{}  {what}",
                humantime::format_rfc3339_seconds(std::time::SystemTime::now())
            );
        }
    };
    loop {
        let output = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log)
            .map_err(|e| format!("opening {}: {e}", log.display()))?;
        let errors = output.try_clone().map_err(|e| e.to_string())?;
        let mut command = Command::new(&exe);
        command
            .args(args(url))
            .env("AGENT_GRAPH_HOME", root)
            .env_remove(SUPERVISOR_VAR)
            .stdin(std::process::Stdio::null())
            .stdout(output)
            .stderr(errors);
        win::no_window(&mut command);
        match command.status() {
            Ok(status) if status.success() => break,
            Ok(status) => note(&format!(
                "watch-remote stopped ({status}): starting it again in {RESTART_AFTER} seconds."
            )),
            Err(e) => note(&format!(
                "can't run {}: {e}: trying again in {RESTART_AFTER} seconds.",
                exe.display()
            )),
        }
        std::thread::sleep(std::time::Duration::from_secs(RESTART_AFTER.into()));
    }
    // (Unless another has taken its place.)
    if std::fs::read_to_string(&pid_file).ok() == me {
        let _ = std::fs::remove_file(&pid_file);
    }
    Ok(())
}

/// Where the supervisor's process is kept while it runs (Windows).
fn pid_file(root: &Path) -> PathBuf {
    root.join("watch-remote.pid")
}

/// Stops the supervisor in the pid file, and what it's running, if it's
/// running (Windows).
fn stop_running(root: &Path) {
    let pid_file = pid_file(root);
    let Ok(id) = std::fs::read_to_string(&pid_file) else {
        return;
    };
    let id = id.trim();
    // (Only that process: not another that has its pid now.)
    if crate::process::alive(id) == Some(true) {
        if let Some((pid, _)) = id.split_once('@') {
            let mut command = Command::new("taskkill");
            command.args(["/PID", pid, "/T", "/F"]);
            win::no_window(&mut command);
            let _ = command.output();
        }
    }
    let _ = std::fs::remove_file(&pid_file);
}

/// Whether to tell launchd or systemd (not when testing: the tests only
/// check the files).
fn manages_services() -> bool {
    std::env::var_os("AGENT_GRAPH_NO_SERVICE_MANAGER").is_none()
}

/// launchd's domain for this user's login session: `gui/<uid>`.
fn gui_domain() -> Result<String, String> {
    let out = Command::new("id")
        .arg("-u")
        .output()
        .map_err(|e| format!("can't find your user id: {e}"))?;
    let uid = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if uid.is_empty() || !uid.bytes().all(|b| b.is_ascii_digit()) {
        return Err("can't find your user id".into());
    }
    Ok(format!("gui/{uid}"))
}

/// Runs `program` with `args`; what it said if it failed.
fn run(program: &str, args: &[&str]) -> Result<(), String> {
    let out = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("can't run {program}: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let said = String::from_utf8_lossy(&out.stderr);
    Err(format!(
        "`{program} {}` failed: {}",
        args.join(" "),
        said.trim()
    ))
}

/// The Run key's entry, and starting programs without a window (Windows).
#[cfg(windows)]
mod win {
    use std::os::windows::process::CommandExt;
    use std::path::Path;
    use std::process::{Command, Stdio};

    use windows_sys::Win32::Foundation::{
        ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, HANDLE_FLAG_INHERIT, SetHandleInformation,
    };
    use windows_sys::Win32::System::Console::{
        GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
    };
    use windows_sys::Win32::System::Registry::{
        HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
    };
    use windows_sys::Win32::System::Threading::{
        CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW,
    };

    use super::{RUN_KEY, RUN_VALUE, SUPERVISOR_VAR};

    /// The key the entry's in: the Run key, or (for the tests, which mustn't
    /// touch it) `AGENT_GRAPH_RUN_KEY`.
    fn key() -> Vec<u16> {
        let key = std::env::var("AGENT_GRAPH_RUN_KEY")
            .ok()
            .filter(|k| !k.is_empty())
            .unwrap_or_else(|| RUN_KEY.to_string());
        wide(&key)
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    /// What the entry runs, if there is one.
    pub fn run_entry() -> Option<String> {
        let (key, value) = (key(), wide(RUN_VALUE));
        let mut size = 0u32;
        // SAFETY: the names are NUL-terminated; with no buffer, it gives the size.
        let found = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut size,
            )
        };
        if found != ERROR_SUCCESS {
            return None;
        }
        let mut data = vec![0u16; (size as usize).div_ceil(2)];
        // SAFETY: `data` holds `size` bytes.
        let read = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                data.as_mut_ptr().cast(),
                &mut size,
            )
        };
        if read != ERROR_SUCCESS {
            return None;
        }
        let end = data.iter().position(|&c| c == 0).unwrap_or(data.len());
        Some(String::from_utf16_lossy(&data[..end]))
    }

    /// Has the entry run `command` at login.
    pub fn set_run_entry(command: &str) -> Result<(), String> {
        let (key, value, data) = (key(), wide(RUN_VALUE), wide(command));
        // SAFETY: the names and data are NUL-terminated, and the size is the data's, in bytes.
        let set = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                value.as_ptr(),
                REG_SZ,
                data.as_ptr().cast(),
                (data.len() * 2) as u32,
            )
        };
        if set == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(format!(
                "can't add it to your Startup apps (the registry's Run key): {}",
                std::io::Error::from_raw_os_error(set as i32)
            ))
        }
    }

    /// Removes the entry, if it's there.
    pub fn remove_run_entry() -> Result<(), String> {
        let (key, value) = (key(), wide(RUN_VALUE));
        // SAFETY: the names are NUL-terminated.
        let removed =
            unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr()) };
        if removed == ERROR_SUCCESS || removed == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            Err(format!(
                "can't remove it from your Startup apps (the registry's Run key): {}",
                std::io::Error::from_raw_os_error(removed as i32)
            ))
        }
    }

    /// Has `command` run without a window.
    pub fn no_window(command: &mut Command) {
        command.creation_flags(CREATE_NO_WINDOW);
    }

    /// Starts `exe` as the supervisor (`autostarted`), without a window, and
    /// out of the job it's in, if it can be, so it outlasts a terminal that
    /// ends its processes when it closes.
    pub fn start_supervisor(exe: &Path, root: &Path, url: &str) -> Result<(), String> {
        // A child gets every handle that can be inherited, whatever it's
        // given as its own: this one's stdout, say, a pipe whose reader
        // (a terminal, or a tool running this) would then wait as long as
        // the supervisor runs. So this one's aren't, from now on.
        for which in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            // SAFETY: a handle this process has, or none (which is refused).
            unsafe { SetHandleInformation(GetStdHandle(which), HANDLE_FLAG_INHERIT, 0) };
        }
        let start = |flags: u32| {
            Command::new(exe)
                .args(["watch-remote", "--autostarted"])
                .arg(format!("--url={}", url.trim_end_matches('/')))
                .arg("--data-dir")
                .arg(root)
                .env(SUPERVISOR_VAR, "1")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .creation_flags(flags)
                .spawn()
        };
        let flags = CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP;
        start(flags | CREATE_BREAKAWAY_FROM_JOB)
            .or_else(|_| start(flags))
            .map(|_| ())
            .map_err(|e| format!("can't start {}: {e}", exe.display()))
    }
}

/// Where there's no Run key (anywhere but Windows), nothing's used.
#[cfg(not(windows))]
mod win {
    use std::path::Path;
    use std::process::Command;

    const ONLY: &str = "only on Windows";

    pub fn run_entry() -> Option<String> {
        None
    }

    pub fn set_run_entry(_command: &str) -> Result<(), String> {
        Err(ONLY.into())
    }

    pub fn remove_run_entry() -> Result<(), String> {
        Err(ONLY.into())
    }

    pub fn no_window(_command: &mut Command) {}

    pub fn start_supervisor(_exe: &Path, _root: &Path, _url: &str) -> Result<(), String> {
        Err(ONLY.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_service_runs_it_in_the_background_for_the_site() {
        assert_eq!(
            args(crate::remote::DEFAULT_URL),
            ["watch-remote", "--background"]
        );
        assert_eq!(
            args("https://example.test/"),
            ["watch-remote", "--background", "--url=https://example.test"]
        );
    }

    #[test]
    fn the_startup_entry_quotes_each_word_as_windows_splits_them() {
        assert_eq!(
            login_command(
                Path::new(r"C:\Program Files\agent-graph\agent-graph.exe"),
                crate::remote::DEFAULT_URL,
                None
            ),
            r#""C:\Program Files\agent-graph\agent-graph.exe" "watch-remote" "--autostarted""#
        );
        assert_eq!(
            login_command(
                Path::new(r"C:\a.exe"),
                "https://a.test/",
                Some(Path::new(r"C:\my data\"))
            ),
            r#""C:\a.exe" "watch-remote" "--autostarted" "--url=https://a.test" "--data-dir=C:\my data\\""#,
            "a backslash before the closing quote is doubled"
        );
        assert_eq!(windows_arg(r#"say "hi""#), r#""say \"hi\"""#);
        assert_eq!(windows_arg(r#"a\"b"#), r#""a\\\"b""#);
        assert_eq!(
            windows_arg(r"a\b"),
            r#""a\b""#,
            "elsewhere, a backslash is itself"
        );
    }

    #[test]
    fn the_launchd_agent_runs_it_at_login_and_again_only_after_a_failure() {
        let plist = launchd_plist(
            Path::new("/opt/homebrew/bin/agent-graph"),
            &args("https://a.test/?x=1&y=<2>"),
            Path::new("/Users/me/.agent-graph/watch-remote.log"),
            Some(Path::new("/tmp/ag home")),
        );
        assert!(plist.contains(&format!("<string>{LABEL}</string>")));
        assert!(plist.contains("<string>/opt/homebrew/bin/agent-graph</string>"));
        assert!(plist.contains("<string>--background</string>"));
        assert!(plist.contains("<string>--url=https://a.test/?x=1&amp;y=&lt;2&gt;</string>"));
        assert!(plist.contains("<key>RunAtLoad</key>\n  <true/>"));
        assert!(plist.contains("<key>SuccessfulExit</key>\n    <false/>"));
        assert!(plist.contains("<key>AGENT_GRAPH_HOME</key>\n    <string>/tmp/ag home</string>"));
        assert_eq!(
            plist
                .matches("/Users/me/.agent-graph/watch-remote.log")
                .count(),
            2,
            "its output and errors"
        );
        let plain = launchd_plist(
            Path::new("/x"),
            &args(crate::remote::DEFAULT_URL),
            Path::new("/l"),
            None,
        );
        assert!(!plain.contains("EnvironmentVariables"));
    }

    #[test]
    fn the_systemd_service_quotes_what_it_runs() {
        let unit = systemd_unit(
            Path::new("/home/me/my bin/agent-graph"),
            &args("https://a.test/50%"),
            Some(Path::new("/home/me/ag$%")),
        );
        assert!(unit.contains(
            "ExecStart=\"/home/me/my bin/agent-graph\" \"watch-remote\" \"--background\" \"--url=https://a.test/50%%\"\n"
        ));
        assert!(unit.contains("Environment=\"AGENT_GRAPH_HOME=/home/me/ag$%%\"\n"));
        let dollar = systemd_unit(Path::new("/home/me/$bin/agent-graph"), &[], None);
        assert!(dollar.contains("ExecStart=\"/home/me/$$bin/agent-graph\"\n"));
        assert!(unit.contains("Restart=on-failure\n"));
        assert!(unit.contains("WantedBy=default.target\n"));
        let plain = systemd_unit(Path::new("/x"), &args(crate::remote::DEFAULT_URL), None);
        assert!(!plain.contains("Environment="));
    }
}
