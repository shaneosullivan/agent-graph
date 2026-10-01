//! Sessions' names as their agents have them now. Claude Code runs no hook
//! when a session is renamed, so the log only learns a new name at the
//! session's next prompt or turn end. The viewer reads it sooner from the
//! session's transcript, which Claude Code writes it to at once. Codex
//! (the CLI and the ChatGPT app) names a session after its first turn, or
//! when it's renamed, in its `session_index.jsonl`, which is read the same
//! way.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::adapter::{claude_code, codex};
use crate::event::Payload;
use crate::timeline::Timed;

#[derive(Default)]
pub struct Names {
    /// Each open Claude Code session's transcript, and its size when last read.
    watched: BTreeMap<String, (PathBuf, Option<u64>)>,
    /// How many events `watched` was worked out from.
    seen: usize,
    /// Codex's sessions, and its index of their names (with its size when
    /// last read).
    codex: Vec<String>,
    codex_index: Option<(PathBuf, Option<u64>)>,
    /// The latest name read for each session. Kept after a session ends: its
    /// last rename may have come after its last turn.
    pub titles: BTreeMap<String, String>,
}

impl Names {
    /// Re-reads the names of sessions whose transcripts have grown. Returns
    /// whether any name changed.
    pub fn poll(&mut self, events: &[Timed]) -> bool {
        if events.len() != self.seen {
            self.watch(events);
        }
        let mut changed = self.poll_codex();
        for (id, (path, read_at)) in &mut self.watched {
            let size = std::fs::metadata(&*path).ok().map(|m| m.len());
            if size.is_none() || size == *read_at {
                continue;
            }
            *read_at = size;
            if let Some(title) = claude_code::title_of(path) {
                if self.titles.get(id) != Some(&title) {
                    self.titles.insert(id.clone(), title);
                    changed = true;
                }
            }
        }
        changed
    }

    /// Re-reads Codex's index of names, if it's grown, for its sessions.
    fn poll_codex(&mut self) -> bool {
        if self.codex.is_empty() {
            return false;
        }
        let Some((path, read_at)) = &mut self.codex_index else {
            return false;
        };
        let size = std::fs::metadata(&*path).ok().map(|m| m.len());
        if size.is_none() || size == *read_at {
            return false;
        }
        *read_at = size;
        let Some(index) = claude_code::read_file(path, Some(INDEX_TAIL)) else {
            return false;
        };
        let mut changed = false;
        for id in &self.codex {
            let thread = id.strip_prefix("codex:").unwrap_or(id);
            if let Some(title) = codex::title_in(&index, thread) {
                if self.titles.get(id) != Some(&title) {
                    self.titles.insert(id.clone(), title);
                    changed = true;
                }
            }
        }
        changed
    }

    /// Works out which sessions to watch: Claude Code's, with a transcript,
    /// that haven't ended since they last started, and Codex's.
    fn watch(&mut self, events: &[Timed]) {
        let mut open: BTreeMap<&str, Option<String>> = BTreeMap::new();
        let mut codex = std::collections::BTreeSet::new();
        for t in events {
            let e = &t.event;
            if e.node.starts_with("codex:") && !e.node.contains('/') && e.kind == "session.started"
            {
                codex.insert(e.node.clone());
            }
            if !e.node.starts_with("claude-code:") || e.node.contains('/') {
                continue;
            }
            match e.kind.as_str() {
                "session.started" => {
                    if let Payload::SessionStarted(d) = e.payload() {
                        open.insert(&e.node, d.transcript_path);
                    }
                }
                "session.ended" => {
                    open.remove(e.node.as_str());
                }
                _ => {}
            }
        }
        let open: BTreeMap<String, PathBuf> = open
            .into_iter()
            .filter_map(|(id, path)| Some((id.to_string(), PathBuf::from(path?))))
            .collect();
        self.watched
            .retain(|id, (path, _)| open.get(id) == Some(path));
        for (id, path) in open {
            self.watched.entry(id).or_insert((path, None));
        }
        if codex.len() != self.codex.len() {
            // A new session's name may already be in the index: read it again.
            self.codex_index =
                codex::codex_home().map(|home| (home.join("session_index.jsonl"), None));
        }
        self.codex = codex.into_iter().collect();
        self.seen = events.len();
    }
}

/// How much of the end of Codex's index of names to read (as the hooks do).
const INDEX_TAIL: u64 = 256 * 1024;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::Envelope;

    fn event(n: u32, kind: &str, data: serde_json::Value) -> Timed {
        Timed::new(Envelope {
            v: 1,
            id: format!("01K000000000000000000000{n:02}"),
            ts: format!("2026-09-27T11:00:{n:02}.000Z"),
            kind: kind.into(),
            node: "claude-code:s".into(),
            parent: None,
            source: None,
            trace: None,
            data,
        })
    }

    #[test]
    fn a_rename_shows_before_the_session_says_anything() {
        let dir = tempfile::tempdir().unwrap();
        let transcript = dir.path().join("s.jsonl");
        let named = |title: &str| {
            serde_json::json!({"type": "custom-title", "customTitle": title}).to_string() + "\n"
        };
        std::fs::write(&transcript, named("Fix the login bug")).unwrap();
        let started = event(
            1,
            "session.started",
            serde_json::json!({"transcript_path": transcript}),
        );
        let mut names = Names::default();
        let mut events = vec![started];

        assert!(names.poll(&events));
        assert_eq!(names.titles["claude-code:s"], "Fix the login bug");
        // Nothing new written: nothing read, nothing changed.
        assert!(!names.poll(&events));

        std::fs::write(
            &transcript,
            named("Fix the login bug") + &named("Login loop"),
        )
        .unwrap();
        assert!(names.poll(&events));
        assert_eq!(names.titles["claude-code:s"], "Login loop");

        // Ended: no longer watched, but still named.
        events.push(event(2, "session.ended", serde_json::json!({})));
        std::fs::write(&transcript, named("Something else")).unwrap();
        assert!(!names.poll(&events));
        assert_eq!(names.titles["claude-code:s"], "Login loop");
    }
}
