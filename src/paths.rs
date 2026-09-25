//! Where Agent Graph keeps its files, on every platform.

use std::env;
use std::path::PathBuf;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_key_strips_unsafe_characters() {
        assert_eq!(file_key("claude-code", "5f2c-1e"), "claude-code-5f2c-1e");
        assert_eq!(file_key("cursor", "a:b/c\\d"), "cursor-a_b_c_d");
    }
}
