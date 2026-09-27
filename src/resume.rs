//! Reopening a session in the agent that ran it: for Claude Code,
//! `claude --resume <id>` in the session's folder.
//!
//! Only the local viewer offers this (see `timeline::Environment`), and
//! `view::open` runs the command in a new terminal window. Everything here is
//! pure, so the graph can say which sessions can be opened.

use crate::reducer::{Node, NodeKind};

/// A command that reopens a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resume {
    /// The agent, for the button: "Claude Code".
    pub app: &'static str,
    pub program: &'static str,
    pub args: Vec<String>,
    /// The session's folder. Agents file sessions by folder, so the command
    /// has to run there.
    pub cwd: String,
    /// The session hasn't ended, so it's probably still open somewhere.
    /// Resuming it there too would put two processes on one conversation, so
    /// the command opens a copy of it instead.
    pub copy: bool,
    /// The Claude desktop app's own id for the session, when the app has it:
    /// it opens there, as it is, rather than in a terminal. Only the viewer,
    /// which can look at the app's records, sets it.
    pub desktop: Option<String>,
}

/// How to reopen `node`, if it's a session whose agent can do that.
pub fn resume(node: &Node) -> Option<Resume> {
    if node.kind != NodeKind::Session {
        return None;
    }
    let cwd = node.cwd.clone().filter(|c| !c.is_empty())?;
    let copy = node.ended_at.is_none();
    match node.provider.as_str() {
        "claude-code" => {
            let id = session_id(&node.id, "claude-code:")?;
            let mut args = vec!["--resume".to_string(), id.to_string()];
            if copy {
                args.push("--fork-session".to_string());
            }
            Some(Resume {
                app: "Claude Code",
                program: "claude",
                args,
                cwd,
                copy,
                desktop: None,
            })
        }
        _ => None,
    }
}

/// Opening a Claude Code session the Claude desktop app has, by the app's
/// `id` for it: the app needs nothing else, not even its folder.
pub fn in_desktop_app(id: String) -> Resume {
    Resume {
        app: "Claude Code",
        program: "claude",
        args: Vec::new(),
        cwd: String::new(),
        copy: false,
        desktop: Some(id),
    }
}

/// The agent's own id for a session, when it's safe to put on a command line
/// as it is: letters, digits, `-` and `_`, starting with a letter or digit so
/// it can't pass for an option (a crafted log could hold anything).
fn session_id<'a>(node_id: &'a str, prefix: &str) -> Option<&'a str> {
    let id = node_id.strip_prefix(prefix)?;
    let safe = id.len() <= 128
        && id.starts_with(|c: char| c.is_ascii_alphanumeric())
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    safe.then_some(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_session_ids_are_used() {
        let id = "5f2c1e8a-3b4d-4e5f-8a9b-0c1d2e3f4a5b";
        assert_eq!(
            session_id(&format!("claude-code:{id}"), "claude-code:"),
            Some(id)
        );
        for bad in [
            "",
            "a b",
            "a;rm -rf ~",
            "$(x)",
            "a/b",
            "é",
            "-",
            "--dangerously-skip-permissions",
        ] {
            let node = format!("claude-code:{bad}");
            assert_eq!(session_id(&node, "claude-code:"), None, "{bad:?}");
        }
        assert_eq!(session_id("codex:abc", "claude-code:"), None);
    }
}
