//! Keeps an in-memory, time-ordered copy of the events directory, reading
//! only what's new on each poll.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::event::Envelope;
pub use crate::timeline::Timed;

pub struct Tail {
    dir: PathBuf,
    /// Bytes consumed so far from each file; always at a line boundary.
    offsets: BTreeMap<PathBuf, u64>,
    /// Every event, in the order the reducer applies them. Shared, so a
    /// request can take them without copying, and work without the lock;
    /// changing them then makes a copy.
    pub events: Arc<Vec<Timed>>,
    /// Lines that weren't valid events.
    pub skipped: usize,
}

impl Tail {
    pub fn new(dir: &Path) -> Tail {
        Tail {
            dir: dir.to_path_buf(),
            offsets: BTreeMap::new(),
            events: Arc::new(Vec::new()),
            skipped: 0,
        }
    }

    /// Reads anything appended since the last poll. Returns whether the
    /// events changed.
    pub fn poll(&mut self) -> io::Result<bool> {
        let sizes = list(&self.dir)?;
        // A file that shrank or vanished means history was rewritten (e.g. by
        // a cleanup), so start over rather than patch.
        let rewritten = self
            .offsets
            .iter()
            .any(|(path, &offset)| sizes.get(path).is_none_or(|&size| size < offset));
        if rewritten {
            self.offsets.clear();
            self.events = Arc::new(Vec::new());
            self.skipped = 0;
        }

        let mut added = Vec::new();
        for (path, size) in sizes {
            let offset = self.offsets.get(&path).copied().unwrap_or(0);
            if size > offset {
                let consumed = self.read_new(&path, offset, size, &mut added)?;
                self.offsets.insert(path, offset + consumed);
            } else {
                self.offsets.entry(path).or_insert(offset);
            }
        }

        if !added.is_empty() {
            let events = Arc::make_mut(&mut self.events);
            events.extend(added.into_iter().map(Timed::new));
            // Nearly always already in order, which this sort handles in linear time.
            crate::timeline::sort(events);
            return Ok(true);
        }
        Ok(rewritten)
    }

    /// Parses the complete lines between `offset` and `size`, returning how
    /// many bytes were consumed. A trailing partial line is left for later.
    fn read_new(
        &mut self,
        path: &Path,
        offset: u64,
        size: u64,
        out: &mut Vec<Envelope>,
    ) -> io::Result<u64> {
        let mut file = File::open(path)?;
        file.seek(SeekFrom::Start(offset))?;
        let mut buf = Vec::with_capacity((size - offset) as usize);
        file.take(size - offset).read_to_end(&mut buf)?;
        let Some(end) = buf.iter().rposition(|&b| b == b'\n') else {
            return Ok(0);
        };
        for line in buf[..end].split(|&b| b == b'\n') {
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            match serde_json::from_slice::<Envelope>(line) {
                Ok(event) => out.push(event),
                Err(_) => self.skipped += 1,
            }
        }
        Ok(end as u64 + 1)
    }
}

/// The size of each `*.jsonl` file in `dir`. A missing directory is empty.
fn list(dir: &Path) -> io::Result<BTreeMap<PathBuf, u64>> {
    let mut sizes = BTreeMap::new();
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(sizes),
        Err(e) => return Err(e),
    };
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "jsonl") {
            sizes.insert(path, entry.metadata()?.len());
        }
    }
    Ok(sizes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::append;

    fn line(id: &str, secs: u32) -> String {
        format!(
            r#"{{"v":1,"id":"{id}","ts":"2026-09-25T10:00:{secs:02}.000Z","type":"status","node":"x:1","data":{{"state":"idle"}}}}"#
        ) + "\n"
    }

    #[test]
    fn reads_only_whole_new_lines() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("x-1.jsonl");
        let mut tail = Tail::new(dir.path());
        assert!(!tail.poll().unwrap(), "nothing there yet");

        append(&file, line("01A", 1).as_bytes()).unwrap();
        assert!(tail.poll().unwrap());
        assert_eq!(tail.events.len(), 1);
        assert!(!tail.poll().unwrap(), "no change");

        // Half a line: wait for the rest.
        let second = line("01B", 2);
        let (head, rest) = second.split_at(20);
        append(&file, head.as_bytes()).unwrap();
        assert!(!tail.poll().unwrap());
        append(&file, rest.as_bytes()).unwrap();
        assert!(tail.poll().unwrap());
        assert_eq!(tail.events.len(), 2);
        assert_eq!(tail.skipped, 0);
    }

    #[test]
    fn keeps_events_in_time_order_across_files() {
        let dir = tempfile::tempdir().unwrap();
        let mut tail = Tail::new(dir.path());
        append(&dir.path().join("a.jsonl"), line("01C", 5).as_bytes()).unwrap();
        tail.poll().unwrap();
        append(&dir.path().join("b.jsonl"), line("01D", 3).as_bytes()).unwrap();
        tail.poll().unwrap();
        let ids: Vec<_> = tail.events.iter().map(|t| t.event.id.as_str()).collect();
        assert_eq!(ids, ["01D", "01C"]);
    }

    #[test]
    fn starts_over_when_a_file_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        let mut tail = Tail::new(dir.path());
        append(&dir.path().join("a.jsonl"), line("01E", 1).as_bytes()).unwrap();
        append(&dir.path().join("b.jsonl"), line("01F", 2).as_bytes()).unwrap();
        tail.poll().unwrap();
        fs::remove_file(dir.path().join("a.jsonl")).unwrap();
        assert!(tail.poll().unwrap());
        let ids: Vec<_> = tail.events.iter().map(|t| t.event.id.as_str()).collect();
        assert_eq!(ids, ["01F"]);
    }
}
