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

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::resume::Resume;

/// Runs `r` in a new terminal window.
pub fn in_terminal(r: &Resume) -> Result<(), String> {
    if cfg!(target_os = "macos") {
        macos(r)
    } else if cfg!(windows) {
        windows(r)
    } else {
        linux(r)
    }
}

/// The command to type yourself, for when a window can't be opened.
pub fn command_line(r: &Resume) -> String {
    let command = std::iter::once(r.program)
        .chain(r.args.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ");
    if cfg!(windows) {
        format!("cd /d \"{}\" && {command}", r.cwd)
    } else {
        format!("cd {} && {command}", sh_quote(&r.cwd))
    }
}

fn macos(r: &Resume) -> Result<(), String> {
    let id = ulid::Ulid::from_datetime(std::time::SystemTime::now());
    let path = std::env::temp_dir().join(format!("agent-graph-{id}.command"));
    write_script(&path, &macos_script(r))
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

/// A script that deletes itself, runs the agent in the session's folder, then
/// leaves a shell there.
fn macos_script(r: &Resume) -> String {
    let command = std::iter::once(r.program)
        .chain(r.args.iter().map(String::as_str))
        .map(sh_quote)
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "#!/bin/sh\n# Written by agent-graph view to open a session. It deletes itself.\n\
         rm -f -- \"$0\"\ncd -- {} || exit 1\n{command}\nexec \"${{SHELL:-/bin/sh}}\" -l\n",
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

fn windows(r: &Resume) -> Result<(), String> {
    let status = Command::new("cmd")
        .args(windows_args(r))
        .current_dir(&r.cwd)
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
fn windows_args(r: &Resume) -> Vec<String> {
    ["/C", "start", "Agent Graph", "cmd", "/K", r.program]
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

fn linux(r: &Resume) -> Result<(), String> {
    if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return Err("there's no desktop here to open a terminal on".to_string());
    }
    let chosen = std::env::var("TERMINAL").ok().filter(|t| !t.is_empty());
    let (program, before): (String, &[&str]) = match chosen {
        Some(t) => (t, &["-e"]),
        None => TERMINALS
            .iter()
            .find(|(name, _)| on_path(name).is_some())
            .map(|(name, before)| (name.to_string(), *before))
            .ok_or("couldn't find a terminal; set $TERMINAL to yours")?,
    };
    let mut child = Command::new(&program)
        .args(linux_args(before, r))
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

/// `<before> sh -c <script> agent-graph <folder> <command…>`. The script gets
/// the folder and command as arguments, so neither is ever parsed as shell.
fn linux_args(before: &[&str], r: &Resume) -> Vec<String> {
    const SCRIPT: &str = r#"cd -- "$1" && shift && "$@"; exec "${SHELL:-sh}""#;
    before
        .iter()
        .copied()
        .chain(["sh", "-c", SCRIPT, "agent-graph", &r.cwd, r.program])
        .map(String::from)
        .chain(r.args.iter().cloned())
        .collect()
}

fn on_path(program: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|dir| dir.join(program))
        .find(|p| p.is_file())
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
        let script = macos_script(&resume("/w/it's $HOME `x`"));
        assert!(script.starts_with("#!/bin/sh\n"));
        assert!(script.contains("\ncd -- '/w/it'\\''s $HOME `x`' || exit 1\n"));
        assert!(script.contains("\n'claude' '--resume' 'abc-123'\n"));
        assert!(script.contains("rm -f -- \"$0\""));
    }

    #[test]
    fn linux_passes_the_folder_as_an_argument() {
        let args = linux_args(&["--"], &resume("/w/a b;c"));
        assert_eq!(
            args,
            [
                "--",
                "sh",
                "-c",
                r#"cd -- "$1" && shift && "$@"; exec "${SHELL:-sh}""#,
                "agent-graph",
                "/w/a b;c",
                "claude",
                "--resume",
                "abc-123",
            ]
        );
    }

    #[test]
    fn windows_keeps_the_folder_off_the_command_line() {
        let args = windows_args(&resume(r"C:\w\a&b"));
        assert_eq!(
            args,
            [
                "/C",
                "start",
                "Agent Graph",
                "cmd",
                "/K",
                "claude",
                "--resume",
                "abc-123"
            ]
        );
    }

    #[test]
    fn the_command_to_type_yourself() {
        if cfg!(windows) {
            assert_eq!(
                command_line(&resume(r"C:\w\app")),
                r#"cd /d "C:\w\app" && claude --resume abc-123"#
            );
        } else {
            assert_eq!(
                command_line(&resume("/w/my app")),
                "cd '/w/my app' && claude --resume abc-123"
            );
        }
    }
}
