//! Reopening a session in the agent that ran it: for Claude Code,
//! `claude --resume <id>` in the session's folder; for Codex, `codex resume
//! <id>`.
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
    /// The link that shows the session in its agent's desktop app (the
    /// Claude app's, or the ChatGPT app's for Codex), when the app has it:
    /// it opens there, as it is, rather than in a terminal. Only the viewer,
    /// which can look at the apps' records, sets it. Always one
    /// `app_of_link` knows.
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
        // A copy of one still open is a fork.
        "codex" => {
            let id = session_id(&node.id, "codex:")?;
            let verb = if copy { "fork" } else { "resume" };
            Some(Resume {
                app: "Codex",
                program: "codex",
                args: vec![verb.to_string(), id.to_string()],
                cwd,
                copy,
                desktop: None,
            })
        }
        _ => None,
    }
}

/// Opening a session its desktop app has, by the app's `link` for it
/// (`claude_app_link`, `codex_app_link`): the app needs nothing else, not
/// even its folder.
pub fn in_desktop_app(link: String) -> Option<Resume> {
    let (app, program) = if link.starts_with("codex:") {
        ("Codex", "codex")
    } else {
        ("Claude Code", "claude")
    };
    app_of_link(&link)?;
    Some(Resume {
        app,
        program,
        args: Vec::new(),
        cwd: String::new(),
        copy: false,
        desktop: Some(link),
    })
}

/// The link that shows a Claude Code session in the Claude desktop app's
/// Code tab, by the app's own id for it (`local_…`).
pub fn claude_app_link(id: &str) -> Option<String> {
    is_claude_app_id(id).then(|| format!("claude://code/continue?session={id}"))
}

/// The link that shows a Codex thread in the ChatGPT desktop app.
pub fn codex_app_link(thread: &str) -> Option<String> {
    let safe = (1..=128).contains(&thread.len())
        && thread
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    safe.then(|| format!("codex://threads/{thread}"))
}

/// The app a link from `claude_app_link` or `codex_app_link` opens, as the
/// viewer names it; `None` for anything else, which isn't opened. (A link
/// has only letters, digits and `:/?=_-`.)
pub fn app_of_link(link: &str) -> Option<&'static str> {
    if let Some(id) = link.strip_prefix("claude://code/continue?session=") {
        return is_claude_app_id(id).then_some("the Claude app");
    }
    let thread = link.strip_prefix("codex://threads/")?;
    (codex_app_link(thread).as_deref() == Some(link)).then_some("the ChatGPT app")
}

/// Whether `id` is one of the Claude app's own session ids (`local_…`), as
/// its link takes them. (A record could hold anything; this goes in a URL.)
pub fn is_claude_app_id(id: &str) -> bool {
    id.strip_prefix("local_").is_some_and(|rest| {
        (1..=64).contains(&rest.len())
            && rest.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    })
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

    #[test]
    fn only_the_apps_own_links_are_opened() {
        let claude = claude_app_link("local_776bf0af-e7ae").unwrap();
        assert_eq!(claude, "claude://code/continue?session=local_776bf0af-e7ae");
        assert_eq!(app_of_link(&claude), Some("the Claude app"));
        let codex = codex_app_link("01a0f497-0990").unwrap();
        assert_eq!(codex, "codex://threads/01a0f497-0990");
        assert_eq!(app_of_link(&codex), Some("the ChatGPT app"));
        for bad in [
            "claude://code/continue?session=local_a&calc",
            "claude://code/continue?session=e3db28e0",
            "codex://threads/a b",
            "codex://threads/",
            "codex://threads/x?view=review",
            "https://example.com",
        ] {
            assert_eq!(app_of_link(bad), None, "{bad:?}");
        }
        assert_eq!(claude_app_link("local_a&calc"), None);
        assert_eq!(codex_app_link("a;b"), None);
        assert!(in_desktop_app("codex://threads/x y".into()).is_none());
    }
}
