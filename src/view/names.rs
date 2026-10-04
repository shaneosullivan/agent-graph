//! Sessions' names as their agents have them now. Claude Code runs no hook
//! when a session is renamed, so the log only learns a new name at the
//! session's next prompt or turn end. The viewer reads it sooner from the
//! session's transcript, which Claude Code writes it to at once. Codex
//! (the CLI and the ChatGPT app) names a session after its first turn, or
//! when it's renamed, in its `session_index.jsonl`, which is read the same
//! way. Cursor names a chat during its first turn, or when it's renamed: a
//! CLI chat in its `meta.json`, an app chat in the app's database (see
//! `adapter::cursor::title_of`), both read when they change.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::SystemTime;

use crate::adapter::{claude_code, codex, cursor};
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
    /// Cursor's chats (their conversation ids), each CLI chat's `meta.json`
    /// once found, with when it was last read, and when the app's database
    /// was last read.
    cursor: Vec<String>,
    cursor_meta: BTreeMap<String, (PathBuf, Option<SystemTime>)>,
    cursor_store: Option<(PathBuf, Option<SystemTime>)>,
    /// The most recently active of Cursor's chats, whose saved state (todo
    /// list, plan waiting) is read again when the app's database changes.
    cursor_recent: Vec<String>,
    /// What Cursor has saved of those chats: the app in its database, the
    /// CLI in each chat's own (the todo list only).
    pub cursor_chats: BTreeMap<String, cursor::AppChat>,
    /// When each CLI chat's database was last read.
    cursor_cli_stores: BTreeMap<String, Option<SystemTime>>,
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
        changed |= self.poll_cursor();
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

    /// Re-reads Cursor's chats' names where they've changed: a CLI chat's
    /// `meta.json`, and the app's database (which changes all the time while
    /// the app's open, so it's read about once a poll).
    fn poll_cursor(&mut self) -> bool {
        if self.cursor.is_empty() {
            return false;
        }
        let Some(home) = crate::paths::user_home() else {
            return false;
        };
        let chats = home.join(".cursor").join("chats");
        let mut found = BTreeMap::new();
        for id in &self.cursor {
            let meta = self.cursor_meta.get(id).cloned().or_else(|| {
                let conversation = id.strip_prefix("cursor:").unwrap_or(id);
                let path = cursor::cli_chat_dir(&chats, conversation)?.join("meta.json");
                path.exists().then_some((path, None))
            });
            if let Some(meta) = meta {
                found.insert(id.clone(), meta);
            }
        }
        let mut changed = false;
        let mut set = |titles: &mut BTreeMap<String, String>, id: &str, title: String| {
            if titles.get(id) != Some(&title) {
                titles.insert(id.to_string(), title);
                changed = true;
            }
        };
        for (id, (path, read_at)) in &mut found {
            let modified = modified(path);
            if modified.is_none() || modified == *read_at {
                continue;
            }
            *read_at = modified;
            let conversation = id.strip_prefix("cursor:").unwrap_or(id);
            if let Some(title) = cursor::cli_title(&chats, conversation) {
                set(&mut self.titles, id, title);
            }
        }
        self.cursor_meta = found;
        // The app's chats: those without a CLI chat's file.
        let store = self
            .cursor_store
            .get_or_insert_with(|| (cursor::app_store(&home).unwrap_or_default(), None));
        let wal = store.0.with_extension("vscdb-wal");
        let changed_at = modified(&wal).max(modified(&store.0));
        if changed_at.is_some() && changed_at != store.1 {
            store.1 = changed_at;
            if let Some(names) = cursor::app_titles(&store.0, None) {
                for id in self
                    .cursor
                    .iter()
                    .filter(|id| !self.cursor_meta.contains_key(*id))
                {
                    let conversation = id.strip_prefix("cursor:").unwrap_or(id);
                    if let Some(title) = names.get(conversation) {
                        set(&mut self.titles, id, title.clone());
                    }
                }
            }
            for id in self
                .cursor_recent
                .iter()
                .filter(|id| !self.cursor_meta.contains_key(*id))
            {
                let conversation = id.strip_prefix("cursor:").unwrap_or(id);
                let chat = cursor::app_chat(&store.0, conversation);
                changed |= update(&mut self.cursor_chats, id, chat);
            }
        }
        // The CLI's chats: each in a database of its own, beside its
        // `meta.json`, read when it changes.
        let mut stores = BTreeMap::new();
        for id in self
            .cursor_recent
            .iter()
            .filter(|id| self.cursor_meta.contains_key(*id))
        {
            let (meta, _) = &self.cursor_meta[id];
            let store = meta.with_file_name("store.db");
            let modified = modified(&store.with_extension("db-wal")).max(modified(&store));
            let read_at = self.cursor_cli_stores.get(id).copied().flatten();
            stores.insert(id.clone(), modified);
            if modified.is_none() || modified == read_at {
                continue;
            }
            let conversation = id.strip_prefix("cursor:").unwrap_or(id);
            let chat = cursor::cli_chat(&chats, conversation).map(|c| cursor::AppChat {
                todos: c.todos,
                ..Default::default()
            });
            changed |= update(&mut self.cursor_chats, id, chat);
        }
        self.cursor_cli_stores = stores;
        // Only the chats still among the most recent.
        let before = self.cursor_chats.len();
        let recent = &self.cursor_recent;
        self.cursor_chats.retain(|id, _| recent.contains(id));
        changed | (self.cursor_chats.len() != before)
    }

    /// Works out which sessions to watch: Claude Code's, with a transcript,
    /// that haven't ended since they last started, and Codex's.
    fn watch(&mut self, events: &[Timed]) {
        let mut open: BTreeMap<&str, Option<String>> = BTreeMap::new();
        let mut codex = std::collections::BTreeSet::new();
        let mut cursor = std::collections::BTreeSet::new();
        // Each of Cursor's chats, by how recently it last did something.
        let mut cursor_last: BTreeMap<&str, usize> = BTreeMap::new();
        for (at, t) in events.iter().enumerate() {
            let e = &t.event;
            if e.node.starts_with("codex:") && !e.node.contains('/') && e.kind == "session.started"
            {
                codex.insert(e.node.clone());
            }
            if e.node.starts_with("cursor:") && !e.node.contains('/') && e.kind == "session.started"
            {
                cursor.insert(e.node.clone());
            }
            if e.node.starts_with("cursor:") && !e.node.contains('/') {
                cursor_last.insert(&e.node, at);
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
        if cursor.len() != self.cursor.len() {
            // A new chat's name may already be in the app's database.
            if let Some(store) = &mut self.cursor_store {
                store.1 = None;
            }
        }
        self.cursor = cursor.into_iter().collect();
        let mut recent: Vec<(&str, usize)> = cursor_last.into_iter().collect();
        recent.sort_by_key(|(_, at)| std::cmp::Reverse(*at));
        self.cursor_recent = recent
            .into_iter()
            .take(CURSOR_WATCHED)
            .map(|(id, _)| id.to_string())
            .collect();
        self.seen = events.len();
    }
}

/// How many of Cursor's most recently active chats have their saved state
/// read: each is a query each time the app's database changes.
const CURSOR_WATCHED: usize = 20;

/// Puts `chat` (what Cursor has saved of chat `id`) in `chats`, or takes
/// the chat out if there's nothing. Returns whether that changed anything.
fn update(
    chats: &mut BTreeMap<String, cursor::AppChat>,
    id: &str,
    chat: Option<cursor::AppChat>,
) -> bool {
    match chat {
        Some(chat) if chats.get(id) != Some(&chat) => {
            chats.insert(id.to_string(), chat);
            true
        }
        Some(_) => false,
        None => chats.remove(id).is_some(),
    }
}

/// When `path` was last modified, if it exists.
fn modified(path: &std::path::Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
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
