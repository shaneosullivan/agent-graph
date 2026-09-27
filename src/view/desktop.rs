//! The Claude desktop app's Claude Code sessions, so the viewer can open a
//! session there rather than in a terminal. The app keeps a record of each
//! in `claude-code-sessions/<account>/<org>/local_<id>.json`, with its own
//! id for it (`sessionId`, `local_…`) and Claude Code's (`cliSessionId`);
//! `claude://code/continue?session=<its own id>` shows it. A session run in
//! a terminal has no record, and an archived one can't be shown that way.
//!
//! The records are rewritten as sessions change, so each poll re-reads only
//! the ones whose modification time moved.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;

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

/// Whether `id` is one of the app's own session ids, as its link takes
/// them. (A record could hold anything; this goes in a URL.)
pub fn is_app_id(id: &str) -> bool {
    id.strip_prefix("local_").is_some_and(|rest| {
        (1..=64).contains(&rest.len())
            && rest.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    })
}

/// A record as last read: when it was changed, and what it says (Claude
/// Code's id and the app's), if it's a session the link can show.
type Record = (Option<SystemTime>, Option<(String, String)>);

pub struct Desktop {
    dir: Option<PathBuf>,
    records: BTreeMap<PathBuf, Record>,
    /// Node id (`claude-code:<Claude Code's id>`) → the app's id.
    pub sessions: BTreeMap<String, String>,
}

impl Desktop {
    /// The app's sessions in `dir` (`sessions_dir()`, or none).
    pub fn new(dir: Option<PathBuf>) -> Desktop {
        Desktop {
            dir,
            records: BTreeMap::new(),
            sessions: BTreeMap::new(),
        }
    }

    /// Re-reads the records that are new or changed. Returns whether the
    /// sessions changed.
    pub fn poll(&mut self) -> bool {
        let Some(dir) = &self.dir else { return false };
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
        let sessions: BTreeMap<String, String> = self
            .records
            .values()
            .filter_map(|(_, s)| s.clone())
            .map(|(cli, app)| (format!("claude-code:{cli}"), app))
            .collect();
        let changed = sessions != self.sessions;
        self.sessions = sessions;
        changed
    }
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
    (is_app_id(app) && !cli.is_empty()).then(|| (cli.to_string(), app.to_string()))
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
        let mut desktop = Desktop::new(Some(dir.path().to_path_buf()));
        assert!(desktop.poll());
        // Not the archived one, nor one whose id isn't the app's kind.
        assert_eq!(
            desktop.sessions,
            BTreeMap::from([("claude-code:abc".to_string(), "local_1".to_string())])
        );
        assert!(!desktop.poll(), "nothing changed");

        std::fs::remove_file(dir.path().join("account/org/local_1.json")).unwrap();
        assert!(desktop.poll());
        assert!(desktop.sessions.is_empty());
    }

    #[test]
    fn no_folder_no_sessions() {
        let mut desktop = Desktop::new(None);
        assert!(!desktop.poll());
        let mut missing = Desktop::new(Some(PathBuf::from("/nonexistent/claude")));
        assert!(!missing.poll());
    }

    #[test]
    fn only_the_apps_own_ids_are_used() {
        assert!(is_app_id("local_776bf0af-e7ae-448e-a3a0-c2324c9b5d85"));
        for bad in [
            "",
            "local_",
            "776bf0af",
            "local_a b",
            "local_a&b",
            "local_a/b",
        ] {
            assert!(!is_app_id(bad), "{bad:?}");
        }
    }
}
