//! Sessions' names as their agents have them now. Claude Code runs no hook
//! when a session is renamed, so the log only learns a new name at the
//! session's next prompt or turn end. The viewer reads it sooner from the
//! session's transcript, which Claude Code writes it to at once.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::adapter::claude_code;
use crate::event::Payload;
use crate::timeline::Timed;

#[derive(Default)]
pub struct Names {
    /// Each open Claude Code session's transcript, and its size when last read.
    watched: BTreeMap<String, (PathBuf, Option<u64>)>,
    /// How many events `watched` was worked out from.
    seen: usize,
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
        let mut changed = false;
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

    /// Works out which sessions to watch: Claude Code's, with a transcript,
    /// that haven't ended since they last started.
    fn watch(&mut self, events: &[Timed]) {
        let mut open: BTreeMap<&str, Option<String>> = BTreeMap::new();
        for t in events {
            let e = &t.event;
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
        self.seen = events.len();
    }
}

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
