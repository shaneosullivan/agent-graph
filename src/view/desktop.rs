//! The sessions desktop apps have, so the viewer can open a session there
//! rather than in a terminal.
//!
//! - The Claude app's Claude Code sessions. The app keeps a record of each
//!   in `claude-code-sessions/<account>/<org>/local_<id>.json`, with its own
//!   id for it (`sessionId`, `local_…`) and Claude Code's (`cliSessionId`);
//!   `claude://code/continue?session=<its own id>` shows it. A session run
//!   in a terminal has no record, and an archived one can't be shown that
//!   way. The records are rewritten as sessions change, so each poll
//!   re-reads only the ones whose modification time moved.
//! - The ChatGPT app's Codex sessions: those it started, whose rollout (the
//!   session's `transcript_path`) says so in its first line (`originator`).
//!   `codex://threads/<thread id>` shows one. Sessions run in a terminal
//!   stay there: one still open would have two Codex processes on it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;

use crate::event::Payload;
use crate::resume;
use crate::timeline::Timed;

/// Where the app keeps its session records, on this computer's OS.
pub fn sessions_dir() -> Option<PathBuf> {
    let base = if cfg!(target_os = "macos") {
        crate::paths::user_home()?.join("Library/Application Support")
    } else if cfg!(windows) {
        PathBuf::from(std::env::var_os("APPDATA")?)
    } else {
        return None; // The app isn't made for Linux.
    };
    Some(base.join("Claude").join("claude-code-sessions"))
}

/// What the ChatGPT app calls itself in the rollouts of the Codex sessions
/// it runs.
const CODEX_DESKTOP: &str = "Codex Desktop";

/// A record as last read: when it was changed, and what it says (Claude
/// Code's id and the app's), if it's a session the link can show.
type Record = (Option<SystemTime>, Option<(String, String)>);

pub struct Desktop {
    dir: Option<PathBuf>,
    records: BTreeMap<PathBuf, Record>,
    /// Whether the ChatGPT app is installed, to open Codex sessions in.
    chatgpt: bool,
    /// Codex sessions whose rollouts have been read: whether the app ran it.
    codex: BTreeMap<String, bool>,
    /// How many events `codex` was worked out from.
    seen: usize,
    /// Node id → the link that shows it in its app (see `resume::Resume::desktop`).
    pub sessions: BTreeMap<String, String>,
}

impl Desktop {
    /// The Claude app's sessions in `dir` (`sessions_dir()`, or none), and
    /// the ChatGPT app's, if `chatgpt` (`chatgpt_installed()`).
    pub fn new(dir: Option<PathBuf>, chatgpt: bool) -> Desktop {
        Desktop {
            dir,
            records: BTreeMap::new(),
            chatgpt,
            codex: BTreeMap::new(),
            seen: 0,
            sessions: BTreeMap::new(),
        }
    }

    /// Re-reads the Claude app's records that are new or changed, and the
    /// rollouts of Codex sessions new in `events`. Returns whether the
    /// sessions changed.
    pub fn poll(&mut self, events: &[Timed]) -> bool {
        self.read_codex(events);
        self.read_records();
        let sessions: BTreeMap<String, String> = self
            .records
            .values()
            .filter_map(|(_, s)| s.clone())
            .filter_map(|(cli, app)| {
                Some((format!("claude-code:{cli}"), resume::claude_app_link(&app)?))
            })
            .chain(
                self.codex
                    .iter()
                    .filter(|(_, app)| **app)
                    .filter_map(|(id, _)| {
                        let thread = id.strip_prefix("codex:")?;
                        Some((id.clone(), resume::codex_app_link(thread)?))
                    }),
            )
            .collect();
        let changed = sessions != self.sessions;
        self.sessions = sessions;
        changed
    }

    /// Reads, once, whether each Codex session new in `events` was the app's.
    fn read_codex(&mut self, events: &[Timed]) {
        if !self.chatgpt || events.len() == self.seen {
            return;
        }
        // (Events can land out of order, so all of them: only new sessions'
        // rollouts are read.)
        for t in events {
            let e = &t.event;
            if e.kind != "session.started"
                || !e.node.starts_with("codex:")
                || e.node.contains('/')
                || self.codex.contains_key(&e.node)
            {
                continue;
            }
            let Payload::SessionStarted(d) = e.payload() else {
                continue;
            };
            let app = d
                .transcript_path
                .and_then(|p| crate::adapter::codex::session_meta(Path::new(&p)))
                .is_some_and(|meta| {
                    meta.get("originator").and_then(Value::as_str) == Some(CODEX_DESKTOP)
                });
            self.codex.insert(e.node.clone(), app);
        }
        self.seen = events.len();
    }

    fn read_records(&mut self) {
        let Some(dir) = &self.dir else { return };
        let mut seen = BTreeMap::new();
        for path in records_in(dir) {
            let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
            let entry = match self.records.remove(&path) {
                Some((at, session)) if at == modified && at.is_some() => (at, session),
                _ => (modified, read(&path)),
            };
            seen.insert(path, entry);
        }
        self.records = seen;
    }
}

/// Whether the ChatGPT desktop app is installed, to open Codex sessions in.
pub fn chatgpt_installed() -> bool {
    cfg!(target_os = "macos") && crate::paths::chatgpt_apps().iter().any(|a| a.is_dir())
}

/// `dir/<account>/<org>/local_*.json`.
fn records_in(dir: &Path) -> Vec<PathBuf> {
    let children = |dir: &Path| -> Vec<PathBuf> {
        std::fs::read_dir(dir)
            .map(|entries| entries.flatten().map(|e| e.path()).collect())
            .unwrap_or_default()
    };
    children(dir)
        .iter()
        .filter(|p| p.is_dir())
        .flat_map(|account| children(account))
        .filter(|p| p.is_dir())
        .flat_map(|org| children(&org))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("local_") && n.ends_with(".json"))
        })
        .collect()
}

/// Claude Code's id and the app's, from a record the link can show.
fn read(path: &Path) -> Option<(String, String)> {
    let record: Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    if record.get("isArchived").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let app = record.get("sessionId")?.as_str()?;
    let cli = record.get("cliSessionId")?.as_str()?;
    (resume::is_claude_app_id(app) && !cli.is_empty()).then(|| (cli.to_string(), app.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, json: &str) {
        let org = dir.join("account").join("org");
        std::fs::create_dir_all(&org).unwrap();
        std::fs::write(org.join(name), json).unwrap();
    }

    #[test]
    fn finds_the_apps_sessions_and_notices_changes() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "local_1.json",
            r#"{"sessionId":"local_1","cliSessionId":"abc","isArchived":false}"#,
        );
        write(
            dir.path(),
            "local_2.json",
            r#"{"sessionId":"local_2","cliSessionId":"def","isArchived":true}"#,
        );
        write(
            dir.path(),
            "local_3.json",
            r#"{"sessionId":"local_3; rm -rf ~","cliSessionId":"ghi"}"#,
        );
        write(
            dir.path(),
            "other.json",
            r#"{"sessionId":"local_4","cliSessionId":"jkl"}"#,
        );
        let mut desktop = Desktop::new(Some(dir.path().to_path_buf()), false);
        assert!(desktop.poll(&[]));
        // Not the archived one, nor one whose id isn't the app's kind.
        assert_eq!(
            desktop.sessions,
            BTreeMap::from([(
                "claude-code:abc".to_string(),
                "claude://code/continue?session=local_1".to_string()
            )])
        );
        assert!(!desktop.poll(&[]), "nothing changed");

        std::fs::remove_file(dir.path().join("account/org/local_1.json")).unwrap();
        assert!(desktop.poll(&[]));
        assert!(desktop.sessions.is_empty());
    }

    #[test]
    fn no_folder_no_sessions() {
        let mut desktop = Desktop::new(None, false);
        assert!(!desktop.poll(&[]));
        let mut missing = Desktop::new(Some(PathBuf::from("/nonexistent/claude")), false);
        assert!(!missing.poll(&[]));
    }

    /// The app's Codex sessions open there; the CLI's don't, nor any when
    /// the app isn't installed.
    #[test]
    fn the_chatgpt_apps_codex_sessions_open_there() {
        let dir = tempfile::tempdir().unwrap();
        let rollout = |name: &str, originator: &str| {
            let path = dir.path().join(name);
            let meta = serde_json::json!({
                "timestamp": "2026-10-01T00:00:00.000Z",
                "type": "session_meta",
                "payload": {"id": name, "originator": originator, "base_instructions": "x".repeat(50_000)}
            });
            std::fs::write(&path, format!("{meta}\n{{\"type\":\"event_msg\"}}\n")).unwrap();
            path
        };
        let started = |node: &str, path: &Path| -> Timed {
            let line = serde_json::json!({
                "v": 1, "id": format!("01K{node}"), "ts": "2026-10-01T00:00:00.000Z",
                "type": "session.started", "node": node,
                "data": {"transcript_path": path}
            });
            Timed::new(serde_json::from_value(line).unwrap())
        };
        let events = vec![
            started("codex:app-1", &rollout("app-1", "Codex Desktop")),
            started("codex:cli-1", &rollout("cli-1", "codex_exec")),
            started("codex:gone-1", &dir.path().join("missing.jsonl")),
        ];
        let mut desktop = Desktop::new(None, true);
        assert!(desktop.poll(&events));
        assert_eq!(
            desktop.sessions,
            BTreeMap::from([(
                "codex:app-1".to_string(),
                "codex://threads/app-1".to_string()
            )])
        );
        assert!(!desktop.poll(&events));
        let mut without = Desktop::new(None, false);
        assert!(!without.poll(&events));
    }
}
