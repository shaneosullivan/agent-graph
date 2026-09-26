//! Where Agent Graph keeps its files, on every platform.

use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// The user's home directory: `USERPROFILE` on Windows, `HOME` elsewhere.
pub fn user_home() -> Option<PathBuf> {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(env::home_dir)
}

/// The data directory: `$AGENT_GRAPH_HOME`, or `~/.agent-graph`.
pub fn data_dir() -> Option<PathBuf> {
    if let Some(dir) = env::var_os("AGENT_GRAPH_HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    user_home().map(|home| home.join(".agent-graph"))
}

pub fn events_dir(root: &std::path::Path) -> PathBuf {
    root.join("events")
}

pub fn raw_dir(root: &std::path::Path) -> PathBuf {
    root.join("raw")
}

pub fn state_file(root: &std::path::Path) -> PathBuf {
    root.join("state.json")
}

pub fn emit_log(root: &std::path::Path) -> PathBuf {
    root.join("emit.log")
}

/// A file name for one provider session, safe on every platform
/// (Windows rejects `:` and several other characters).
pub fn file_key(provider: &str, session: &str) -> String {
    let clean = |s: &str| -> String {
        s.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect()
    };
    format!("{}-{}", clean(provider), clean(session))
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

#[cfg(test)]
mod tests {
    use super::*;
    /// R8, R11: the agent is found on PATH by full path; a relative entry (like
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

    #[test]
    fn file_key_strips_unsafe_characters() {
        assert_eq!(file_key("claude-code", "5f2c-1e"), "claude-code-5f2c-1e");
        assert_eq!(file_key("cursor", "a:b/c\\d"), "cursor-a_b_c_d");
    }
}
