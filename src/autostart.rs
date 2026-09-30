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
//!
//! Either runs `agent-graph watch-remote --background` by the path that
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

/// The arguments the service runs `agent-graph` with, for the site `url`.
pub fn args(url: &str) -> Vec<String> {
    let url = url.trim_end_matches('/');
    let mut args = vec!["watch-remote".to_string(), "--background".to_string()];
    if url != crate::remote::DEFAULT_URL {
        args.push(format!("--url={url}"));
    }
    args
}

/// Where the service's file goes, on this system; an error where it isn't
/// supported yet.
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
    let path = service_path()?;
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
    let contents = if cfg!(target_os = "macos") {
        launchd_plist(&exe, &args, &log, home.as_deref())
    } else {
        systemd_unit(&exe, &args, home.as_deref())
    };
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
pub fn disable() -> Result<(), String> {
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
    service_path().is_ok_and(|p| p.exists())
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
