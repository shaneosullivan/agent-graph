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

/// A record as last read: when it was changed, and what it says, if it's a
/// session the link can show.
type Record = (Option<SystemTime>, Option<AppSession>);

/// What the Claude app's record of a session says.
#[derive(Clone, Debug, PartialEq)]
struct AppSession {
    /// Claude Code's id for it.
    cli: String,
    /// The app's own (`local_…`).
    app: String,
    /// What it needs you to do, when the app's summary of its latest turn
    /// says it's blocked on you ("unlock Mac, then run …").
    blocked: Option<String>,
}

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
            .filter_map(|(_, s)| s.as_ref())
            .filter_map(|s| {
                Some((
                    format!("claude-code:{}", s.cli),
                    resume::claude_app_link(&s.app)?,
                ))
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

    /// Records, in the events in `events_dir`, that the Claude app's
    /// sessions it says are blocked on you are: the app works that out
    /// after each turn (`postTurnSummary`: "blocked", with what you need to
    /// do), after its hooks have run, so they'd only say the session's
    /// idle. A session is marked once, while its last word in the log is
    /// that it's idle: your next prompt sets it working, as ever. Returns
    /// how many were marked. (Both the viewer and `watch-remote` do this,
    /// so the site shows it too.)
    pub fn flag_blocked(&mut self, events_dir: &Path) -> usize {
        self.read_records();
        let blocked: Vec<(String, String)> = self
            .records
            .values()
            .filter_map(|(_, s)| s.as_ref())
            .filter_map(|s| Some((s.cli.clone(), s.blocked.clone()?)))
            .collect();
        blocked
            .into_iter()
            .filter(|(cli, needs)| mark_blocked(events_dir, cli, needs))
            .count()
    }

    /// Re-reads the Claude app's records that are new or changed.
    pub fn read_records(&mut self) {
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

/// What a record says, if it's a session the app's link can show.
fn read(path: &Path) -> Option<AppSession> {
    let record: Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    if record.get("isArchived").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let app = record.get("sessionId")?.as_str()?;
    let cli = record.get("cliSessionId")?.as_str()?;
    if !resume::is_claude_app_id(app) || !safe_cli_id(cli) {
        return None;
    }
    // Only the summary of its latest turn says where it stands now.
    let summary = record
        .get("postTurnSummary")
        .filter(|_| record.get("postTurnSummaryFor") == record.get("lastAssistantUuid"));
    let text = |key: &str| {
        summary
            .and_then(|s| s.get(key))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|t| !t.is_empty())
    };
    let blocked = (text("status_category") == Some("blocked"))
        .then(|| text("needs_action").or_else(|| text("status_detail")))
        .flatten()
        .map(|t| crate::event::truncate_chars(t, 200));
    Some(AppSession {
        cli: cli.to_string(),
        app: app.to_string(),
        blocked,
    })
}

/// A Claude Code session id fit for a node id and a file name.
fn safe_cli_id(id: &str) -> bool {
    (1..=128).contains(&id.len())
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Marks Claude Code session `cli` as waiting on you (`needs`), in its
/// events file in `events_dir`, if the log's last word on it is that it's
/// idle. Whether it did.
fn mark_blocked(events_dir: &Path, cli: &str, needs: &str) -> bool {
    use crate::adapter::Draft;
    use crate::event::{Payload, Source, State, Status};
    let node = format!("claude-code:{cli}");
    let file = events_dir.join(format!(
        "{}.jsonl",
        crate::paths::file_key("claude-code", cli)
    ));
    let Some(tail) = crate::adapter::claude_code::read_file(&file, Some(256 * 1024)) else {
        return false;
    };
    // The session's own last event that says how it stands (its agents'
    // don't).
    let last = tail.lines().rev().find_map(|line| {
        let event: Value = serde_json::from_str(line).ok()?;
        let kind = event.get("type")?.as_str()?;
        (event.get("node")?.as_str()? == node
            && matches!(kind, "status" | "session.started" | "session.ended"))
        .then_some(event)
    });
    let idle = last.is_some_and(|e| {
        e.get("type").and_then(Value::as_str) == Some("status")
            && e.pointer("/data/state").and_then(Value::as_str) == Some("idle")
    });
    if !idle {
        return false;
    }
    let draft = Draft::new(
        &node,
        Payload::Status(Status {
            state: State::InputRequired,
            summary: Some(needs.to_string()),
            title: None,
        }),
    );
    let source = Source {
        provider: "claude-code".into(),
        provider_version: None,
        adapter: Some("claude-app@1".into()),
    };
    let lines: String = crate::emit::stamp(vec![draft], &source, SystemTime::now())
        .iter()
        .map(|e| crate::emit::to_line(e) + "\n")
        .collect();
    crate::store::append(&file, lines.as_bytes()).is_ok()
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

    /// The app's summary of a session's latest turn saying it's blocked on
    /// you marks it so in its log, once, while it's idle; not for an older
    /// turn's summary, nor one that isn't blocked.
    #[test]
    fn a_session_the_app_says_is_blocked_needs_you() {
        let dir = tempfile::tempdir().unwrap();
        let events = tempfile::tempdir().unwrap();
        let record = |name: &str, cli: &str, category: &str, latest: bool| {
            let json = serde_json::json!({
                "sessionId": name, "cliSessionId": cli,
                "lastAssistantUuid": "u2",
                "postTurnSummaryFor": if latest { "u2" } else { "u1" },
                "postTurnSummary": {
                    "status_category": category,
                    "status_detail": "Mac screen locked",
                    "needs_action": "unlock Mac, then run the release"
                }
            });
            write(dir.path(), &format!("{name}.json"), &json.to_string());
        };
        let log = |cli: &str, state: &str| {
            let line = serde_json::json!({
                "v": 1, "id": "01A", "ts": "2026-10-01T16:52:11.000Z", "type": "status",
                "node": format!("claude-code:{cli}"), "data": {"state": state}
            });
            std::fs::write(
                events.path().join(format!("claude-code-{cli}.jsonl")),
                format!("{line}\n"),
            )
            .unwrap();
        };
        record("local_1", "blocked-idle", "blocked", true);
        log("blocked-idle", "idle");
        record("local_2", "blocked-working", "blocked", true);
        log("blocked-working", "working");
        record("local_3", "old-summary", "blocked", false);
        log("old-summary", "idle");
        record("local_4", "completed", "completed", true);
        log("completed", "idle");

        let mut desktop = Desktop::new(Some(dir.path().to_path_buf()), false);
        assert_eq!(desktop.flag_blocked(events.path()), 1);
        let text =
            std::fs::read_to_string(events.path().join("claude-code-blocked-idle.jsonl")).unwrap();
        let marked: Value = serde_json::from_str(text.lines().last().unwrap()).unwrap();
        assert_eq!(marked["type"], "status");
        assert_eq!(marked["node"], "claude-code:blocked-idle");
        assert_eq!(marked["data"]["state"], "input_required");
        assert_eq!(
            marked["data"]["summary"],
            "unlock Mac, then run the release"
        );
        // Once: it isn't idle any more.
        assert_eq!(desktop.flag_blocked(events.path()), 0);
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
