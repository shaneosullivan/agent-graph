//! Reading and writing the event files.
//!
//! Each session appends to its own `.jsonl` file. Every hook invocation writes
//! its lines with a single append, so concurrent hooks don't interleave lines
//! and no locking is needed.

use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;

use crate::event::Envelope;

/// Creates `path` and its parents. On Unix they're readable only by the current
/// user; on Windows the user profile's ACLs already do that.
pub fn ensure_dir(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(path)
    }
}

/// Appends `bytes` to `path` in one write, creating the file if needed.
pub fn append(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut opts = OpenOptions::new();
    opts.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path)?.write_all(bytes)
}

/// Writes `bytes` to `path` via a temporary file and a rename, so readers never
/// see a half-written file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)
}

#[derive(Debug, Default)]
pub struct Loaded {
    pub events: Vec<Envelope>,
    /// Lines that weren't valid events, e.g. a line cut short by a crash.
    pub skipped_lines: usize,
}

/// Reads every `*.jsonl` file in `dir`. A missing directory is just empty.
pub fn load_events(dir: &Path) -> io::Result<Loaded> {
    let mut loaded = Loaded::default();
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(loaded),
        Err(e) => return Err(e),
    };
    for entry in entries {
        let path = entry?.path();
        if path.extension().is_none_or(|ext| ext != "jsonl") {
            continue;
        }
        for line in BufReader::new(fs::File::open(&path)?).lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<Envelope>(&line) {
                Ok(event) => loaded.events.push(event),
                Err(_) => loaded.skipped_lines += 1,
            }
        }
    }
    Ok(loaded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_skips_bad_lines_and_other_files() {
        let dir = tempfile::tempdir().unwrap();
        let good = r#"{"v":1,"id":"01J8ZK3V7Q9R2M5X4T6W8Y0B1C","ts":"2026-09-25T10:00:00.000Z","type":"status","node":"x:1","data":{"state":"idle"}}"#;
        append(
            &dir.path().join("x-1.jsonl"),
            format!("{good}\n{{\"v\":1,\"id\n").as_bytes(),
        )
        .unwrap();
        fs::write(dir.path().join("notes.txt"), "ignored").unwrap();

        let loaded = load_events(dir.path()).unwrap();
        assert_eq!(loaded.events.len(), 1);
        assert_eq!(loaded.skipped_lines, 1);
    }

    #[test]
    fn missing_dir_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = load_events(&dir.path().join("nope")).unwrap();
        assert!(loaded.events.is_empty());
    }
}
