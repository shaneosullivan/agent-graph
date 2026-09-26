//! `agent-graph watch-remote`: share the event log on the Agent Graph site
//! and keep it up to date.
//!
//! The protocol is built so the site does as little work as possible:
//!
//! 1. `POST /api/logs` with the first chunk creates the log. The reply has
//!    its id, its public URL, and a write token.
//! 2. Every later chunk is `POST /api/logs/<id>/append?offset=<bytes sent so
//!    far>` with `Authorization: Bearer <token>`. The site checks the token
//!    by recomputing an HMAC (no database read) and stores the chunk as one
//!    document keyed by its offset, so it never reads or rewrites what's
//!    already there. (A log with no new events for a week is deleted; what's
//!    sent to it then is refused, and this stops.) A failed chunk is retried with exactly the same bytes
//!    (the site may have stored it); the site keeps the first copy, and
//!    refuses different bytes at an offset it already has.
//!
//! Bodies are raw JSON Lines, never wrapped in JSON, and are cut only at line
//! boundaries.
//!
//! The site keeps only a log's last two keyframes' worth (see `keyframe`):
//! what's shared starts at the log's last keyframe but one, a keyframe
//! follows every `keyframe::EVERY` events, each starting a chunk, and once
//! one is sent, the site is asked to delete what's before the one before
//! (`POST /api/logs/<id>/trim?before=<its offset>`, with the token).

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::event::Envelope;
use crate::keyframe;
use crate::reducer::{self, KEYFRAME_PART, Replay};

pub const DEFAULT_URL: &str = "https://agentgraph.chofter.com";
/// The most sent in one request; the site rejects bigger bodies.
pub const MAX_CHUNK: usize = 256 * 1024;
/// How often to look for new lines.
const POLL: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(60);
/// How often, at most, to work out again which files a shared session's
/// tree spans (only when a file outside it has changed).
const RECHECK: Duration = Duration::from_secs(3);

pub struct Options {
    pub url: String,
    /// `Some("")` means "no password", even if a default is saved.
    pub password: Option<String>,
    pub save_default_password: bool,
    /// Share one session instead of everything.
    pub session: Option<String>,
}

pub fn run(root: &Path, opts: Options) -> Result<(), String> {
    let config = root.join("remote.json");
    if opts.save_default_password {
        match opts.password.as_deref() {
            None => return Err(
                "--save-default-password needs --password=<password> (or --password= to clear it)"
                    .into(),
            ),
            Some("") => {
                clear_default_password(&config)?;
                eprintln!("Cleared the saved default password.");
            }
            Some(p) => {
                save_default_password(&config, p)?;
                eprintln!(
                    "Saved the password as the default for watch-remote ({}).",
                    config.display()
                );
            }
        }
    }
    let password = match opts.password {
        Some(p) => Some(p).filter(|p| !p.is_empty()),
        None => load_default_password(&config)?,
    };

    let base = opts.url.trim_end_matches('/').to_string();
    if password.is_some() && base.starts_with("http://") && !is_local(&base) {
        return Err(format!(
            "{base} isn't HTTPS, so the password would be sent in the clear. Use an https:// URL."
        ));
    }

    let events = crate::paths::events_dir(root);
    let (only, what) = match &opts.session {
        Some(want) => {
            let (session, what) = pick_session(&events, want)?;
            (Some(session), what)
        }
        None => (None, format!("every session in {}", root.display())),
    };
    let mut source = Lines::new(&events, only);
    let mut stream = Stream::start(
        source
            .poll()
            .map_err(|e| format!("reading {}: {e}", events.display()))?,
        keyframe::EVERY,
    );

    let client = Client::new(&base);
    let first = stream.next_len(MAX_CHUNK);
    let log = client
        .create(&stream.pending[..first], password.as_deref())
        .map_err(|e| e.message())?;
    // The link first, so it can be shared straight away.
    println!("{}", log.url);
    io::stdout().flush().ok();
    let who = if password.is_some() {
        "Viewers need the password"
    } else {
        "Anyone with the link can view it"
    };
    eprintln!("Sharing {what}, live. {who}. Press Ctrl+C to stop.");

    stream.sent(first, 0);
    let mut sent = first as u64;
    let mut backoff = Duration::ZERO;
    let mut reading_failed = false;
    // The length of a chunk that failed: it's retried with exactly the same
    // bytes, whatever has arrived since, as the site may have stored it.
    let mut retrying: Option<usize> = None;
    let mut trims = Trims::default();
    loop {
        while !stream.pending.is_empty() {
            let n = retrying.unwrap_or_else(|| stream.next_len(MAX_CHUNK));
            match client.append(&log, sent, &stream.pending[..n]) {
                Ok(()) => {
                    retrying = None;
                    // As soon as a keyframe is stored, the site can let go of
                    // what's before the last (so it never holds three).
                    if let Some(before) = stream.sent(n, sent) {
                        trims.ask(before, &client, &log)?;
                    }
                    sent += n as u64;
                    if !backoff.is_zero() {
                        eprintln!("Reconnected; caught up.");
                        backoff = Duration::ZERO;
                    }
                }
                Err(SendError::Fatal(e) | SendError::Expired(e)) => return Err(e),
                Err(SendError::Retry(e)) => {
                    retrying = Some(n);
                    if backoff.is_zero() {
                        eprintln!("Couldn't reach {base}: {e}. Keeping new events and retrying…");
                    }
                    backoff = (backoff * 2).clamp(Duration::from_secs(2), MAX_BACKOFF);
                    std::thread::sleep(backoff);
                    break;
                }
            }
        }
        trims.retry(&client, &log)?;
        std::thread::sleep(POLL);
        // A read error (say, the directory is briefly missing) is retried,
        // and said once.
        match source.poll() {
            Ok(new) => {
                stream.push(&new);
                if stream.recut {
                    stream.recut(source.read_all().ok());
                }
                reading_failed = false;
            }
            Err(e) => {
                if !reading_failed {
                    eprintln!("Can't read {} ({e}); trying again.", events.display());
                    reading_failed = true;
                }
            }
        }
    }
}

// ---------- keyframes ----------

/// Asking the site to delete what's before a keyframe.
#[derive(Default)]
struct Trims {
    /// Where to trim before, if a request is still to be made.
    pending: Option<u64>,
    /// After a failure, when to ask again, and how long it waited last.
    at: Option<Instant>,
    backoff: Duration,
    /// The site can't (an older copy): it keeps everything sent.
    refused: bool,
}

impl Trims {
    /// Asks now (or once it can), to delete what's before `before`.
    fn ask(&mut self, before: u64, client: &Client, log: &Created) -> Result<(), String> {
        self.pending = Some(before);
        self.retry(client, log)
    }

    /// Makes the request still to be made, if it's time. An error if the
    /// log's gone, which ends the share.
    fn retry(&mut self, client: &Client, log: &Created) -> Result<(), String> {
        let Some(before) = self.pending.filter(|_| !self.refused) else {
            return Ok(());
        };
        if self.at.is_some_and(|at| Instant::now() < at) {
            return Ok(());
        }
        match client.trim(log, before) {
            Ok(()) => {
                self.pending = None;
                self.at = None;
                self.backoff = Duration::ZERO;
            }
            Err(SendError::Retry(_)) => {
                self.backoff = (self.backoff * 2).clamp(Duration::from_secs(30), MAX_BACKOFF * 5);
                self.at = Some(Instant::now() + self.backoff);
            }
            // It keeps everything; sharing carries on.
            Err(SendError::Fatal(e)) => {
                eprintln!(
                    "The site can't trim the log ({e}), so it'll keep everything sent from here on."
                );
                self.refused = true;
            }
            Err(SendError::Expired(e)) => return Err(e),
        }
        Ok(())
    }
}

/// Where an event sorts.
type Key = (std::time::SystemTime, String);

/// A state a keyframe was cut at, to work out later ones from.
struct Checkpoint {
    /// The last event it stands for, and where that sorts (none: before
    /// everything).
    last: Option<(Key, Envelope)>,
    replay: Replay,
    /// How many of the share's events had come when it was cut.
    arrived: usize,
}

/// How many checkpoints are kept. An event that comes later than this many
/// keyframes after where it sorts has the whole log read again (see
/// `Stream::recut`).
const CHECKPOINTS: usize = 3;

/// What's to be sent: the log's last two keyframes' worth to start with,
/// then new lines as they come, with a keyframe after every `every` events,
/// each starting a chunk.
struct Stream {
    every: usize,
    pending: Vec<u8>,
    /// Where keyframes start in `pending` (a chunk starts at each), and
    /// whether the site should start there (see `next_keyframe`).
    breaks: VecDeque<(usize, bool)>,
    /// The events since the oldest checkpoint, as they came: where each
    /// sorts, and its line.
    history: VecDeque<(Key, Vec<u8>)>,
    /// How many events came before `history`'s first.
    dropped: usize,
    /// The states at the last few keyframes, oldest first.
    checkpoints: VecDeque<Checkpoint>,
    /// Events since the last keyframe.
    since: usize,
    /// The earliest an event has come, since the last keyframe, that sorts
    /// before it (from a session joining a shared tree, say).
    late: Option<Key>,
    /// One has sorted before every checkpoint: the next keyframe needs the
    /// whole log, read again (`recut`).
    recut: bool,
    /// Where on the site the last keyframe starts.
    last_keyframe: Option<u64>,
}

impl Stream {
    /// The log so far, cut to its last two keyframes' worth, in the order
    /// its events apply in (`keyframe::Trimmer`).
    fn start(raw: Vec<u8>, every: usize) -> Stream {
        let (trimmed, text) = keyframe::Trimmer::new(valid_utf8(raw), every).finish();
        let lines = &trimmed.lines;
        let checkpoint = |at: usize, replay: Replay| {
            let last = at.checked_sub(1).map(|i| {
                let line = &text[lines[i].at.clone()];
                (
                    lines[i].key.clone(),
                    serde_json::from_slice(line).expect("read before"),
                )
            });
            Checkpoint {
                last,
                replay,
                arrived: at,
            }
        };
        let mut checkpoints = VecDeque::from([checkpoint(trimmed.from, trimmed.at_from)]);
        if trimmed.last > trimmed.from {
            checkpoints.push_back(checkpoint(trimmed.last, trimmed.replay));
        }
        let history = lines[trimmed.from..]
            .iter()
            .map(|line| {
                let mut bytes = text[line.at.clone()].to_vec();
                if !bytes.ends_with(b"\n") {
                    bytes.push(b'\n');
                }
                (line.key.clone(), bytes)
            })
            .collect();
        Stream {
            every,
            breaks: trimmed.keyframes.iter().map(|&k| (k, false)).collect(),
            pending: trimmed.text,
            history,
            dropped: trimmed.from,
            checkpoints,
            since: lines.len() - trimmed.last,
            late: None,
            recut: false,
            last_keyframe: None,
        }
    }

    /// A keyframe to send, starting a chunk. If the site should start there
    /// (`start_here`), it says so to readers already reading, too.
    fn add_keyframe(&mut self, parts: &[Envelope], start_here: bool) {
        if parts.is_empty() {
            return;
        }
        self.breaks.push_back((self.pending.len(), start_here));
        for part in parts {
            let mut part = part.clone();
            if start_here {
                part.data["restart"] = serde_json::Value::Bool(true);
            }
            self.pending
                .extend(serde_json::to_vec(&part).expect("serializable"));
            self.pending.push(b'\n');
        }
    }

    /// New lines, with a keyframe after every `every` events. A poll reads
    /// its files one after another, so its events are sorted first.
    fn push(&mut self, raw: &[u8]) {
        let raw = valid_utf8(raw.to_vec());
        let mut events = Vec::new();
        for line in lines_of(&raw) {
            match serde_json::from_slice::<Envelope>(line) {
                Ok(event) if reducer::is_keyframe(&event) => {}
                Ok(event) => events.push((reducer::sort_key(&event), line)),
                // Not an event: sent as it is, for what it's worth.
                Err(_) => self.pending.extend_from_slice(line),
            }
        }
        events.sort_by(|a, b| a.0.cmp(&b.0));
        for (key, line) in events {
            self.pending.extend_from_slice(line);
            let newest = self.checkpoints.back().and_then(|c| c.last.as_ref());
            if newest.is_some_and(|(last, _)| key < *last) {
                self.late = Some(
                    self.late
                        .take()
                        .map_or(key.clone(), |late| late.min(key.clone())),
                );
            }
            self.history.push_back((key, line.to_vec()));
            self.since += 1;
            if self.since == self.every {
                self.since = 0;
                self.next_keyframe();
            }
        }
    }

    /// A keyframe of everything so far, worked out from the last one and
    /// the events since, as the site would. After a late event, it's worked
    /// out from the newest checkpoint before where that sorts, so it has it
    /// in its place; the site, whose last keyframe doesn't, is told to start
    /// at this one. (One later than every checkpoint is placed where it
    /// came.)
    fn next_keyframe(&mut self) {
        let newest = self.checkpoints.len() - 1;
        let (from, start_here) = match &self.late {
            Some(late) => {
                let Some(i) = self
                    .checkpoints
                    .iter()
                    .rposition(|c| c.last.as_ref().is_none_or(|(key, _)| key < late))
                else {
                    self.recut = true;
                    return;
                };
                // Those after it haven't it in its place either.
                self.checkpoints.truncate(i + 1);
                (i, true)
            }
            None => (newest, false),
        };
        let checkpoint = &self.checkpoints[from];
        let mut replay = checkpoint.replay.clone();
        let mut order: Vec<&(Key, Vec<u8>)> = self
            .history
            .iter()
            .skip(checkpoint.arrived - self.dropped)
            .collect();
        order.sort_by(|a, b| a.0.cmp(&b.0));
        let mut last = self.checkpoints.back().and_then(|c| c.last.clone());
        for (key, line) in order {
            let event: Envelope = serde_json::from_slice(line).expect("read before");
            replay.apply(&event);
            if last.as_ref().is_none_or(|(latest, _)| key > latest) {
                last = Some((key.clone(), event));
            }
        }
        let Some(last) = last else {
            return;
        };
        let parts = replay.keyframe(&last.1, KEYFRAME_PART);
        self.add_keyframe(&parts, start_here);
        self.checkpoints.push_back(Checkpoint {
            last: Some(last),
            replay,
            arrived: self.dropped + self.history.len(),
        });
        while self.checkpoints.len() > CHECKPOINTS {
            self.checkpoints.pop_front();
        }
        let keep = self.checkpoints[0].arrived;
        while self.dropped < keep {
            self.history.pop_front();
            self.dropped += 1;
        }
        self.late = None;
    }

    /// A keyframe of the whole log (`all`: everything read so far, of the
    /// files shared), after an event that sorts before every checkpoint: so
    /// it has it in its place, and the site starts there. Then it's the only
    /// checkpoint. If the log couldn't be read again, it's placed where it
    /// came, as the site would.
    fn recut(&mut self, all: Option<Vec<u8>>) {
        self.recut = false;
        let Some((replay, last)) = all.and_then(|all| keyframe::replay_all(valid_utf8(all))) else {
            self.late = None;
            self.next_keyframe();
            return;
        };
        self.add_keyframe(&replay.keyframe(&last, KEYFRAME_PART), true);
        let arrived = self.dropped + self.history.len();
        self.checkpoints = VecDeque::from([Checkpoint {
            last: Some((reducer::sort_key(&last), last)),
            replay,
            arrived,
        }]);
        self.history.clear();
        self.dropped = arrived;
        self.late = None;
    }

    /// How much of `pending` to send next: whole lines, at most `max`, and
    /// not past the next keyframe.
    fn next_len(&self, max: usize) -> usize {
        let limit = self
            .breaks
            .iter()
            .map(|&(b, _)| b)
            .find(|&b| b > 0)
            .unwrap_or(self.pending.len());
        chunk_len(&self.pending[..limit], max)
    }

    /// `n` bytes of `pending` were stored at `offset`. If they started with
    /// a keyframe, returns where the site can delete what's before: the
    /// keyframe before it, or this one, if the site should start there.
    fn sent(&mut self, n: usize, offset: u64) -> Option<u64> {
        let start_here = match self.breaks.front() {
            Some(&(0, start_here)) => {
                self.breaks.pop_front();
                Some(start_here)
            }
            _ => None,
        };
        self.pending.drain(..n);
        for (b, _) in &mut self.breaks {
            *b -= n;
        }
        let start_here = start_here?;
        let before = self.last_keyframe.replace(offset);
        match start_here {
            true => Some(offset),
            false => before.filter(|&before| before > 0),
        }
    }
}

/// `raw` as valid UTF-8 (anything that isn't, replaced), so the site stores
/// exactly the bytes sent, and chunks' offsets stay true.
fn valid_utf8(raw: Vec<u8>) -> Vec<u8> {
    match String::from_utf8(raw) {
        Ok(text) => text.into_bytes(),
        Err(e) => String::from_utf8_lossy(e.as_bytes())
            .into_owned()
            .into_bytes(),
    }
}

/// The lines in `raw`, each with its newline.
fn lines_of(raw: &[u8]) -> impl Iterator<Item = &[u8]> {
    raw.split_inclusive(|&b| b == b'\n')
        .filter(|line| line.iter().any(|b| !b.is_ascii_whitespace()))
}

// ---------- reading new lines ----------

/// Tracks how far each events file has been read and returns only complete
/// new lines, as raw bytes.
pub struct Lines {
    dir: PathBuf,
    /// Sharing one session: only the files of its tree.
    tree: Option<Tree>,
    offsets: BTreeMap<PathBuf, u64>,
    /// Files that couldn't be read (each said once, until it can be again).
    unreadable: BTreeSet<PathBuf>,
}

/// A shared session and the files its tree spans: its own, and those of
/// sessions (and `run`s) linked under it, which each have their own.
struct Tree {
    root: String,
    root_file: PathBuf,
    files: BTreeSet<PathBuf>,
    /// Sizes of the other files, to notice one changing (a new session
    /// starting under the root, say).
    others: BTreeMap<PathBuf, u64>,
    stale: bool,
    checked: Option<Instant>,
    /// The log couldn't be read at the last recheck (said once).
    failing: bool,
    /// Files the last recheck couldn't read, so left out. One may belong
    /// once it can be read, though its size hasn't changed.
    left_out: BTreeSet<PathBuf>,
    /// How many times the tree has been worked out (each a full reduce).
    rechecks: u32,
}

impl Tree {
    /// Notes whether any file outside the tree has changed, or one left out
    /// can be read again.
    fn notice(&mut self, paths: &[PathBuf]) {
        for path in paths.iter().filter(|p| !self.files.contains(*p)) {
            let size = fs::metadata(path).map(|m| m.len()).ok();
            if size != self.others.get(path).copied() {
                self.stale = true;
                match size {
                    Some(size) => self.others.insert(path.clone(), size),
                    None => self.others.remove(path),
                };
            }
        }
        if self.left_out.iter().any(|p| readable_again(p)) {
            self.stale = true;
        }
    }

    /// Works out the tree's files again. Returns the files it had to leave
    /// out (a member among them drops out of the tree: under-sharing is the
    /// safe side), or `None` if the folder can't be read just now, when it
    /// stays stale, to try again.
    fn recheck(&mut self, dir: &Path) -> Option<Vec<PathBuf>> {
        self.checked = Some(Instant::now());
        self.rechecks += 1;
        match tree_files(dir, &self.root) {
            Ok((files, unreadable)) => {
                self.files = files;
                self.stale = false;
                self.failing = false;
                self.left_out = unreadable.iter().cloned().collect();
                Some(unreadable)
            }
            Err(e) => {
                if !self.failing {
                    eprintln!(
                        "Can't read the events to see which sessions belong to this one ({e}); \
                         holding back any that may have moved, and trying again."
                    );
                    self.failing = true;
                }
                None
            }
        }
    }
}

/// New whole lines read from one file, not yet sent.
struct Batch {
    path: PathBuf,
    lines: Vec<u8>,
    /// Where the file's offset goes once they're sent.
    offset: u64,
}

impl Lines {
    /// Reads every events file in `dir`, or with `only`, the files of that
    /// session's tree.
    pub fn new(dir: &Path, only: Option<String>) -> Lines {
        let tree = only.map(|root| {
            let root_file = session_path(dir, &root);
            Tree {
                root,
                files: BTreeSet::from([root_file.clone()]),
                root_file,
                others: BTreeMap::new(),
                stale: true,
                checked: None,
                failing: false,
                left_out: BTreeSet::new(),
                rechecks: 0,
            }
        });
        Lines {
            dir: dir.to_path_buf(),
            tree,
            offsets: BTreeMap::new(),
            unreadable: BTreeSet::new(),
        }
    }

    /// The new whole lines, from every file or the shared tree's.
    ///
    /// For a tree, the lines are read first and the tree worked out again
    /// after (only if something may have changed it), so the recheck sees
    /// every line about to be sent: a session that has just restarted under
    /// another parent, and so left, isn't sent.
    pub fn poll(&mut self) -> io::Result<Vec<u8>> {
        let entries = match fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let mut paths = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "jsonl") {
                paths.push(path);
            }
        }
        let mut failed = Vec::new();
        let Some(tree) = &mut self.tree else {
            let batches = read_batches(&paths, &self.offsets, &mut self.unreadable, &mut failed);
            return Ok(self.commit(batches));
        };
        tree.notice(&paths);

        let members: Vec<PathBuf> = paths
            .iter()
            .filter(|p| tree.files.contains(*p))
            .cloned()
            .collect();
        let mut batches = read_batches(&members, &self.offsets, &mut self.unreadable, &mut failed);
        // A member that couldn't be read may have restarted under another
        // parent, taking the sessions under it along.
        let member_unreadable = failed.iter().any(|p| *p != tree.root_file);
        let restarted = batches.iter().any(|b| {
            b.path != tree.root_file && contains(&b.lines, br#""type":"session.started""#)
        });
        let due = tree.checked.is_none_or(|t| t.elapsed() >= RECHECK);
        // A restart is checked straight away, unless the log has been
        // unreadable, when it waits like any other recheck. So is a member
        // that can't be read (it drops out, and the rest carry on), though
        // never more often than that: it may still not be readable.
        let recheck_now =
            (restarted && !tree.failing) || ((restarted || member_unreadable || tree.stale) && due);
        let checked = if recheck_now {
            match tree.recheck(&self.dir) {
                Some(skipped) => {
                    // What the recheck read is what's readable now.
                    self.unreadable.retain(|p| skipped.contains(p));
                    for path in skipped {
                        warn_unreadable(&mut self.unreadable, &path, "left it out");
                    }
                    true
                }
                None => false,
            }
        } else {
            false
        };
        if checked {
            // Files that just joined: read them now, after the recheck.
            let joined: Vec<PathBuf> = paths
                .iter()
                .filter(|p| tree.files.contains(*p) && !members.contains(p))
                .cloned()
                .collect();
            batches.extend(read_batches(
                &joined,
                &self.offsets,
                &mut self.unreadable,
                &mut failed,
            ));
        } else if restarted || member_unreadable {
            // Can't tell whether a restarted (or unreadable) session left,
            // taking the sessions under it along: hold back all but the
            // root's lines until we can.
            batches.retain(|b| b.path == tree.root_file);
        }
        let files = tree.files.clone();
        batches.retain(|b| files.contains(&b.path));
        Ok(self.commit(batches))
    }

    /// Everything read so far, of the files shared now: each from its start
    /// up to where it's been read to.
    pub fn read_all(&self) -> io::Result<Vec<u8>> {
        let mut out = Vec::new();
        for (path, &offset) in &self.offsets {
            if self.tree.as_ref().is_some_and(|t| !t.files.contains(path)) {
                continue;
            }
            File::open(path)?.take(offset).read_to_end(&mut out)?;
        }
        Ok(out)
    }

    /// The batches' lines, in order, with their files' offsets moved on.
    fn commit(&mut self, batches: Vec<Batch>) -> Vec<u8> {
        let mut out = Vec::new();
        for batch in batches {
            out.extend_from_slice(&batch.lines);
            self.offsets.insert(batch.path, batch.offset);
        }
        out
    }
}

/// Says, once, that `path` can't be read (and what's done about it).
fn warn_unreadable(unreadable: &mut BTreeSet<PathBuf>, path: &Path, doing: &str) {
    if unreadable.insert(path.to_path_buf()) {
        eprintln!(
            "Can't read {}; {doing}, and trying it again.",
            path.display()
        );
    }
}

/// New whole lines in each of `paths`, past its offset. A file that can't
/// be read is skipped (said once, and added to `failed`) so the rest are
/// still shared; nothing of it is lost, since its offset doesn't move.
fn read_batches(
    paths: &[PathBuf],
    offsets: &BTreeMap<PathBuf, u64>,
    unreadable: &mut BTreeSet<PathBuf>,
    failed: &mut Vec<PathBuf>,
) -> Vec<Batch> {
    let mut batches = Vec::new();
    for path in paths {
        let offset = offsets.get(path).copied().unwrap_or(0);
        match read_new(path, offset) {
            Ok((lines, offset)) => {
                unreadable.remove(path);
                batches.push(Batch {
                    path: path.clone(),
                    lines,
                    offset,
                });
            }
            // Deleted since it was listed: nothing to say.
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                unreadable.remove(path);
            }
            Err(_) => {
                warn_unreadable(unreadable, path, "sharing the rest");
                failed.push(path.clone());
            }
        }
    }
    batches
}

/// Whether a file that couldn't be read can be now (or has gone), so a
/// recheck would see it differently.
fn readable_again(path: &Path) -> bool {
    match File::open(path).and_then(|mut f| f.read(&mut [0; 1])) {
        Ok(_) => true,
        Err(e) => e.kind() == io::ErrorKind::NotFound,
    }
}

/// The whole lines in `path` past `offset`, and the offset after them.
fn read_new(path: &Path, mut offset: u64) -> io::Result<(Vec<u8>, u64)> {
    let size = fs::metadata(path)?.len();
    // A file that shrank was rewritten; send it again. The site drops
    // events it has already seen.
    if size < offset {
        offset = 0;
    }
    let mut lines = Vec::new();
    if size > offset {
        let mut file = File::open(path)?;
        file.seek(SeekFrom::Start(offset))?;
        file.take(size - offset).read_to_end(&mut lines)?;
        // Only whole lines; the rest waits for the next poll.
        let end = lines.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
        lines.truncate(end);
        offset += end as u64;
    }
    Ok((lines, offset))
}

/// How many bytes of `buf` to send next: at most `max`, ending at a line
/// boundary (a single overlong line goes alone).
pub fn chunk_len(buf: &[u8], max: usize) -> usize {
    if buf.len() <= max {
        return buf.len();
    }
    match buf[..max].iter().rposition(|&b| b == b'\n') {
        Some(i) => i + 1,
        None => buf
            .iter()
            .position(|&b| b == b'\n')
            .map_or(buf.len(), |i| i + 1),
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// The events file of `session` (a session's node id).
fn session_path(dir: &Path, session: &str) -> PathBuf {
    let (provider, id) = session.split_once(':').unwrap_or(("", session));
    dir.join(format!("{}.jsonl", crate::paths::file_key(provider, id)))
}

/// The files of every session (and `run`) in `root`'s tree, and the files
/// that couldn't be read (left out: a member among them drops out).
fn tree_files(dir: &Path, root: &str) -> io::Result<(BTreeSet<PathBuf>, Vec<PathBuf>)> {
    let loaded = crate::store::load_events(dir)?;
    let unreadable = loaded.unreadable;
    let graph = crate::reducer::reduce(loaded.events, &crate::reducer::Options::default());
    let root_file = session_path(dir, root);
    let mut files = BTreeSet::from([root_file.clone()]);
    let mut seen = BTreeSet::new();
    let mut queue = vec![root.to_string()];
    while let Some(id) = queue.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        let session = id.split('/').next().unwrap_or(&id);
        let file = session_path(dir, session);
        // A session that couldn't be read may have moved, taking the
        // sessions under it along: none of them is assumed to still belong.
        if file != root_file && unreadable.contains(&file) {
            continue;
        }
        files.insert(file);
        if let Some(node) = graph.nodes.get(&id) {
            queue.extend(node.children.iter().cloned());
        }
    }
    Ok((files, unreadable))
}

/// The session matching `want`, and how to describe it. "current" must be
/// the session this runs in: anything looser (the newest session, say)
/// could publish another project's.
fn pick_session(events: &Path, want: &str) -> Result<(String, String), String> {
    let loaded = crate::store::load_events(events).map_err(|e| e.to_string())?;
    let graph = crate::reducer::reduce(loaded.events, &crate::reducer::Options::default());
    let node = if want == "current" {
        crate::cli::running_in(&graph).ok_or(
            "can't tell which session this is, because it isn't recorded (Agent Graph \
             records Claude Code sessions started after it's installed), so nothing was \
             shared. Choose a session yourself with --session <id>; don't guess.",
        )?
    } else if !loaded.unreadable.is_empty() && !graph.nodes.contains_key(want) {
        // A file that couldn't be read may hold another session it matches.
        let files: Vec<String> = loaded
            .unreadable
            .iter()
            .map(|p| crate::render::clean(&p.display().to_string()))
            .collect();
        return Err(format!(
            "couldn't read {}, so a session there may match {:?}; nothing was shared. \
             Give the session's whole id (with its provider, like claude-code:<id>), or, \
             if the session is in that file, make it readable and try again.",
            files.join(", "),
            crate::render::clean(want),
        ));
    } else {
        // Sessions only: a short prefix that happens to match an agent in
        // another project's session mustn't share that session.
        crate::cli::find_session(&graph, want)?
    };
    let session = node.split('/').next().unwrap_or(&node).to_string();
    if !session.contains(':') {
        return Err(format!(
            "{session:?} isn't a session id Agent Graph records"
        ));
    }
    let folder = graph.nodes.get(&session).and_then(|n| n.cwd.clone());
    // Both come from the log: cleaned, like everything else printed from it.
    let clean = crate::render::clean;
    let what = match folder {
        Some(folder) => format!("session {} (in {})", clean(&session), clean(&folder)),
        None => format!("session {}", clean(&session)),
    };
    Ok((session, what))
}

// ---------- talking to the site ----------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Created {
    pub id: String,
    pub url: String,
    pub write_token: String,
}

pub enum SendError {
    /// Worth trying again: the network, or the site having a bad moment.
    Retry(String),
    /// Won't work however often it's tried (bad token, deleted log, …).
    Fatal(String),
    /// The log's gone: the site deletes one with no new events for a week.
    Expired(String),
}

impl SendError {
    fn message(self) -> String {
        match self {
            SendError::Retry(m) | SendError::Fatal(m) | SendError::Expired(m) => m,
        }
    }
}

pub struct Client {
    agent: ureq::Agent,
    base: String,
}

impl Client {
    pub fn new(base: &str) -> Client {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .http_status_as_error(false)
            .user_agent(concat!("agent-graph/", env!("CARGO_PKG_VERSION")))
            .build();
        Client {
            agent: ureq::Agent::new_with_config(config),
            base: base.trim_end_matches('/').to_string(),
        }
    }

    pub fn create(&self, body: &[u8], password: Option<&str>) -> Result<Created, SendError> {
        let mut req = self
            .agent
            .post(format!("{}/api/logs", self.base))
            .header("Content-Type", "application/x-ndjson")
            .header("X-Agent-Graph-Source", "watch");
        if let Some(p) = password {
            // Headers must be ASCII, so the password travels base64url-encoded.
            req = req.header("X-Agent-Graph-Password", base64url(p.as_bytes()));
        }
        let mut res = req
            .send(body)
            .map_err(|e| SendError::Retry(e.to_string()))?;
        let status = res.status().as_u16();
        let text = res.body_mut().read_to_string().unwrap_or_default();
        match status {
            200 | 201 => serde_json::from_str(&text)
                .map_err(|e| SendError::Fatal(format!("unexpected reply from the site: {e}"))),
            s if s >= 500 || s == 429 => Err(SendError::Retry(format!("the site returned {s}"))),
            s => Err(SendError::Fatal(format!(
                "the site refused to create the log ({s}): {}",
                text.trim()
            ))),
        }
    }

    /// Asks the site to delete the log's chunks before `before` (where a
    /// keyframe starts).
    pub fn trim(&self, log: &Created, before: u64) -> Result<(), SendError> {
        let mut res = self
            .agent
            .post(format!("{}/api/logs/{}/trim", self.base, log.id))
            .query("before", before.to_string())
            .header("Authorization", format!("Bearer {}", log.write_token))
            .send_empty()
            .map_err(|e| SendError::Retry(e.to_string()))?;
        match res.status().as_u16() {
            200 | 204 => Ok(()),
            s if s >= 500 || s == 429 => Err(SendError::Retry(format!("the site returned {s}"))),
            s => {
                let text = res.body_mut().read_to_string().unwrap_or_default();
                let text = text.trim();
                Err(match s {
                    410 => SendError::Expired(
                        "the site has deleted this log: it had no new events for a week. Start a new share to share again."
                            .to_string(),
                    ),
                    _ => SendError::Fatal(format!("{s}: {text}")),
                })
            }
        }
    }

    pub fn append(&self, log: &Created, offset: u64, body: &[u8]) -> Result<(), SendError> {
        let mut res = self
            .agent
            .post(format!("{}/api/logs/{}/append", self.base, log.id))
            .query("offset", offset.to_string())
            .header("Content-Type", "application/x-ndjson")
            .header("Authorization", format!("Bearer {}", log.write_token))
            .send(body)
            .map_err(|e| SendError::Retry(e.to_string()))?;
        match res.status().as_u16() {
            200 | 201 | 204 => Ok(()),
            s if s >= 500 || s == 429 => Err(SendError::Retry(format!("the site returned {s}"))),
            s => {
                let text = res.body_mut().read_to_string().unwrap_or_default();
                Err(SendError::Fatal(format!(
                    "the site refused new events ({s}): {}",
                    text.trim()
                )))
            }
        }
    }
}

fn is_local(base: &str) -> bool {
    let host = base
        .trim_start_matches("http://")
        .split(['/', ':'])
        .next()
        .unwrap_or("");
    matches!(host, "localhost" | "127.0.0.1" | "[")
}

pub fn base64url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, &b)| n | (b as u32) << (16 - 8 * i));
        for i in 0..=chunk.len() {
            out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    out
}

// ---------- the saved default password ----------

#[derive(Serialize, Deserialize, Default)]
struct Saved {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    password: Option<String>,
}

pub fn load_default_password(path: &Path) -> Result<Option<String>, String> {
    match fs::read_to_string(path) {
        Ok(text) => {
            let saved: Saved = serde_json::from_str(&text)
                .map_err(|e| format!("{} is damaged: {e}", path.display()))?;
            Ok(saved.password.filter(|p| !p.is_empty()))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("reading {}: {e}", path.display())),
    }
}

/// Saves the password in plain text (the site needs it to be sent), in a
/// file only the current user can read.
pub fn save_default_password(path: &Path, password: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        crate::store::ensure_dir(dir).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(&Saved {
        password: Some(password.to_string()),
    })
    .expect("serializable");
    let _ = fs::remove_file(path);
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path)
        .and_then(|mut f| f.write_all(text.as_bytes()))
        .map_err(|e| format!("writing {}: {e}", path.display()))
}

pub fn clear_default_password(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("removing {}: {e}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Event `n`: a session starting, or working, `n` seconds in. Some are
    /// written unusually (spaced out, with a field we don't know), as a
    /// newer writer might.
    fn event(n: usize) -> String {
        let message = format!(r#"{{"message_id":"m{n}","to":"x:s0","summary":"hi"}}"#);
        let (kind, data) = match n % 3 {
            0 => ("session.started", "{}"),
            1 => ("status", r#"{"state":"working"}"#),
            _ => ("message.sent", message.as_str()),
        };
        let line = format!(
            r#"{{"v":1,"id":"01K{n:023}","ts":"2026-09-25T10:{:02}:{:02}.000Z","type":"{kind}","node":"x:s{}","data":{data}}}"#,
            n / 60,
            n % 60,
            n / 7
        );
        if n % 5 == 0 {
            line.replace(r#""v":1,"#, r#""v": 1, "extra": [1], "#) + "\n"
        } else {
            line + "\n"
        }
    }

    fn events(range: std::ops::Range<usize>) -> String {
        range.map(event).collect()
    }

    /// Sends everything pending, a chunk at a time, from `offset`: the
    /// chunks, and the trims asked for.
    fn send_all(stream: &mut Stream, offset: &mut u64) -> (Vec<String>, Vec<u64>) {
        let (mut chunks, mut trims) = (Vec::new(), Vec::new());
        while !stream.pending.is_empty() {
            let n = stream.next_len(MAX_CHUNK);
            chunks.push(String::from_utf8(stream.pending[..n].to_vec()).unwrap());
            trims.extend(stream.sent(n, *offset));
            *offset += n as u64;
        }
        (chunks, trims)
    }

    fn is_keyframe_line(line: &str) -> bool {
        line.contains(r#""type":"keyframe""#)
    }

    /// The graph a log's lines make, as the site makes it: from the
    /// keyframe it starts with, if it does.
    fn as_site(text: &str) -> serde_json::Value {
        let parsed: Vec<Envelope> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        let leading = parsed
            .iter()
            .take_while(|e| reducer::is_keyframe(e))
            .count();
        let base = (leading > 0).then(|| reducer::merge_keyframe(&parsed[..leading]).unwrap());
        let rest = parsed[leading..]
            .iter()
            .filter(|e| !reducer::is_keyframe(e))
            .cloned()
            .collect();
        let opts = reducer::Options {
            now: std::time::UNIX_EPOCH + Duration::from_secs(1_790_000_000),
            stale_after: Duration::from_secs(1800),
        };
        serde_json::to_value(reducer::reduce_from(base.as_ref(), rest, &opts)).unwrap()
    }

    /// Keyframes: the log is shared from its last keyframe but one, each
    /// keyframe starts a chunk, and once one's sent, the site's asked to
    /// delete what's before the one before. What it keeps shows the same.
    #[test]
    fn a_log_is_shared_from_its_last_but_one_keyframe() {
        let mut stream = Stream::start(events(0..25).into_bytes(), 10);
        let mut offset = 0;
        let (chunks, trims) = send_all(&mut stream, &mut offset);
        // A keyframe, events 10 to 19; a keyframe, events 20 to 24.
        assert_eq!(chunks.len(), 2);
        for (chunk, range) in chunks.iter().zip([10..20, 20..25]) {
            let (first, rest) = chunk.split_once('\n').unwrap();
            assert!(is_keyframe_line(first));
            assert_eq!(rest, events(range), "the lines as written");
        }
        assert!(trims.is_empty(), "nothing before the first");
        assert_eq!(as_site(&chunks.concat()), as_site(&events(0..25)));

        // Five more make ten since the last keyframe: another, in a chunk
        // of its own; then the site can let go of what's before the last.
        let second = chunks[0].len() as u64;
        stream.push(events(25..30).as_bytes());
        stream.push(b"not an event\n");
        let (more, trims) = send_all(&mut stream, &mut offset);
        assert_eq!(more.len(), 2);
        assert_eq!(more[0], events(25..30));
        let (keyframe, after) = more[1].split_at(more[1].find("not an event").unwrap());
        assert!(keyframe.lines().all(is_keyframe_line));
        assert_eq!(
            after, "not an event\n",
            "sent, as it was, though not counted"
        );
        assert_eq!(trims, [second]);
        let kept = chunks[1].clone() + &more.concat().replace("not an event\n", "");
        assert_eq!(as_site(&kept), as_site(&events(0..30)));

        // Each keyframe carries on from the last: the site, trimmed to the
        // newest, shows the same.
        stream.push(events(30..40).as_bytes());
        let (last, _) = send_all(&mut stream, &mut offset);
        let newest = last.last().unwrap();
        assert!(newest.lines().all(is_keyframe_line));
        assert_eq!(as_site(newest), as_site(&events(0..40)));
    }

    /// An idle for session `s`, `at` seconds in (between its events), which
    /// sorts before the last keyframe when it comes.
    fn late(at: &str, session: usize) -> String {
        format!(
            r#"{{"v":1,"id":"01KLATE0000000000000000000","ts":"2026-09-25T10:00:{at}Z","type":"status","node":"x:s{session}","data":{{"state":"idle"}}}}"#
        ) + "\n"
    }

    /// An event that sorts before the last keyframe (from a session that's
    /// just joined a shared tree, say) is in its place in the next one,
    /// worked out from the checkpoint before it; and the site, whose last
    /// keyframe hasn't it, is told to start at the new one.
    #[test]
    fn a_late_event_is_in_its_place_in_the_next_keyframe() {
        // Checkpoints after events 9 and 19; session 1's events are 7 to 13.
        let mut stream = Stream::start(events(0..25).into_bytes(), 10);
        let mut offset = 0;
        send_all(&mut stream, &mut offset);
        assert!(stream.late.is_none());
        let idle = late("12.500", 1);
        stream.push(idle.as_bytes());
        assert!(stream.late.is_some(), "it sorts before the last keyframe");
        // Five came after the last keyframe: four more make ten.
        stream.push(events(25..29).as_bytes());
        let before = offset;
        let (chunks, trims) = send_all(&mut stream, &mut offset);
        let keyframe = chunks.last().unwrap();
        assert!(keyframe.lines().all(is_keyframe_line));
        let mut all = events(0..29);
        all.push_str(&idle);
        assert_eq!(as_site(keyframe), as_site(&all), "in its place");
        assert_eq!(as_site(keyframe)["nodes"]["x:s1"]["state"], "working");
        let at = before + chunks[..chunks.len() - 1].concat().len() as u64;
        assert_eq!(trims, [at], "the site starts at the new keyframe");
        assert!(
            keyframe.lines().all(|l| l.contains(r#""restart":true"#)),
            "and so do readers already reading"
        );
        assert!(stream.late.is_none());
    }

    /// Two late events in turn: the checkpoints between the first's and
    /// the keyframe it made haven't it in their place, so the second isn't
    /// worked out from them.
    #[test]
    fn late_events_in_turn_are_each_in_their_place() {
        let mut stream = Stream::start(events(0..25).into_bytes(), 10);
        let mut offset = 0;
        send_all(&mut stream, &mut offset);
        let (first, second) = (late("15.500", 2), late("22.500", 3));
        stream.push(first.as_bytes());
        stream.push(events(25..29).as_bytes());
        send_all(&mut stream, &mut offset);
        stream.push(second.as_bytes());
        stream.push(events(29..38).as_bytes());
        let (chunks, trims) = send_all(&mut stream, &mut offset);
        let keyframe = chunks.last().unwrap();
        assert!(keyframe.lines().all(is_keyframe_line));
        let all = events(0..38) + &first + &second;
        assert_eq!(as_site(keyframe), as_site(&all));
        assert_eq!(as_site(keyframe)["nodes"]["x:s2"]["state"], "working");
        assert_eq!(trims.len(), 1);
    }

    /// One later than every checkpoint has the whole log read again, for a
    /// keyframe with it in its place, which the site starts at; or, if it
    /// can't be read, is placed where it came, as the site would.
    #[test]
    fn a_very_late_event_has_the_log_read_again() {
        for read in [true, false] {
            let mut stream = Stream::start(events(0..25).into_bytes(), 10);
            let mut offset = 0;
            let (first, _) = send_all(&mut stream, &mut offset);
            let idle = late("03.500", 0);
            stream.push(idle.as_bytes());
            stream.push(events(25..29).as_bytes());
            assert!(stream.recut, "no keyframe yet: it needs the whole log");
            let all = events(0..29) + &idle;
            stream.recut(read.then(|| all.clone().into_bytes()));
            assert!(!stream.recut);
            let (chunks, trims) = send_all(&mut stream, &mut offset);
            let keyframe = chunks.last().unwrap();
            assert!(keyframe.lines().all(is_keyframe_line));
            if read {
                assert_eq!(as_site(keyframe), as_site(&all));
                assert!(keyframe.contains(r#""restart":true"#));
                assert_eq!(trims, [offset - keyframe.len() as u64]);
                assert_eq!((stream.checkpoints.len(), stream.history.len()), (1, 0));
            } else {
                assert!(!keyframe.contains("restart"));
                assert_eq!(trims, [first[0].len() as u64], "the keyframe before");
            }
        }
    }

    /// What's been read of the shared files, from their starts: whole lines,
    /// and only the files shared.
    #[test]
    fn everything_read_can_be_read_again() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.jsonl");
        std::fs::write(&a, events(0..2)).unwrap();
        let mut lines = Lines::new(dir.path(), None);
        lines.poll().unwrap();
        crate::store::append(&a, (events(2..3) + "{\"half").as_bytes()).unwrap();
        lines.poll().unwrap();
        assert_eq!(
            String::from_utf8(lines.read_all().unwrap()).unwrap(),
            events(0..3)
        );
        // Sharing a tree, not the files of sessions that have left it.
        let dir = tempfile::tempdir().unwrap();
        let root = start("claude-code:p", "/a");
        std::fs::write(dir.path().join("claude-code-p.jsonl"), &root).unwrap();
        std::fs::write(
            dir.path().join("claude-code-c.jsonl"),
            under("claude-code:c", "claude-code:p", 1),
        )
        .unwrap();
        let mut lines = Lines::new(dir.path(), Some("claude-code:p".into()));
        lines.poll().unwrap();
        lines.tree.as_mut().unwrap().checked = None;
        lines.poll().unwrap();
        let count = |all: Vec<u8>| all.split(|&b| b == b'\n').filter(|l| !l.is_empty()).count();
        assert_eq!(count(lines.read_all().unwrap()), 2);
        lines
            .tree
            .as_mut()
            .unwrap()
            .files
            .retain(|f| !f.ends_with("claude-code-c.jsonl"));
        assert_eq!(String::from_utf8(lines.read_all().unwrap()).unwrap(), root);
    }

    /// However long a share goes on, what's kept to work out keyframes from
    /// is a few keyframes' worth.
    #[test]
    fn a_long_share_keeps_a_few_keyframes_worth() {
        let mut stream = Stream::start(events(0..25).into_bytes(), 10);
        let mut offset = 0;
        for start in (25..2025).step_by(50) {
            stream.push(events(start..start + 50).as_bytes());
            send_all(&mut stream, &mut offset);
            assert!(stream.checkpoints.len() <= CHECKPOINTS);
            assert!(
                stream.history.len() <= (CHECKPOINTS + 1) * 10,
                "{}",
                stream.history.len()
            );
        }
        assert_eq!(stream.dropped + stream.history.len(), 2025);
    }

    /// A poll's lines come file by file: its events are sent in order; and
    /// what isn't UTF-8 is replaced, so the site stores the bytes sent.
    #[test]
    fn new_lines_are_sent_in_order_as_utf8() {
        let mut stream = Stream::start(Vec::new(), 10);
        let mut bad = events(2..3).into_bytes();
        bad.extend(b"\xff not an event\n");
        stream.push(&[events(3..4).into_bytes(), events(1..2).into_bytes(), bad].concat());
        let (chunks, _) = send_all(&mut stream, &mut 0);
        let sent = chunks.concat();
        assert_eq!(sent, "\u{fffd} not an event\n".to_string() + &events(1..4));
    }

    #[test]
    fn a_short_log_is_shared_whole() {
        let mut stream = Stream::start((events(0..5) + "not an event\n").into_bytes(), 10);
        let (chunks, trims) = send_all(&mut stream, &mut 0);
        assert_eq!(
            chunks,
            [events(0..5)],
            "as written, but for what isn't an event"
        );
        assert!(trims.is_empty());
        // Past ten, a keyframe follows; there's none before it to trim.
        stream.push(events(5..12).as_bytes());
        let (chunks, trims) = send_all(&mut stream, &mut 0);
        assert_eq!(chunks.len(), 2);
        assert!(chunks[1].lines().next().is_some_and(is_keyframe_line));
        assert!(trims.is_empty());
    }

    fn start(session: &str, cwd: &str) -> String {
        format!(
            r#"{{"v":1,"id":"01K00000000000000000000S01","ts":"2026-09-25T10:00:00.000Z","type":"session.started","node":"{session}","data":{{"cwd":"{cwd}"}}}}"#
        ) + "\n"
    }

    /// R13: a recheck that can't list the folder keeps the tree stale, so
    /// the change that prompted it isn't lost.
    #[cfg(unix)]
    #[test]
    fn a_failed_recheck_tries_again() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("claude-code-p.jsonl"),
            start("claude-code:p", "/a"),
        )
        .unwrap();
        let mut lines = Lines::new(dir.path(), Some("claude-code:p".into()));
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o300)).unwrap();
        let listable = std::fs::read_dir(dir.path()).is_ok(); // as root, it still is
        let tree = lines.tree.as_mut().unwrap();
        let result = tree.recheck(dir.path());
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        if !listable {
            assert!(result.is_none());
            assert!(tree.stale, "gave up on the change");
        }
    }

    /// R15: in a shared tree, a file elsewhere that can't be read doesn't
    /// stop a new session joining (it's left out of the recheck, not fatal).
    #[test]
    fn an_unreadable_file_elsewhere_doesnt_block_the_tree() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("claude-code-p.jsonl"),
            start("claude-code:p", "/a"),
        )
        .unwrap();
        let mut lines = Lines::new(dir.path(), Some("claude-code:p".into()));
        assert_eq!(
            lines
                .poll()
                .unwrap()
                .split(|&b| b == b'\n')
                .filter(|l| !l.is_empty())
                .count(),
            1
        );
        std::fs::create_dir(dir.path().join("unrelated.jsonl")).unwrap();
        std::fs::write(
            dir.path().join("claude-code-c.jsonl"),
            under("claude-code:c", "claude-code:p", 1),
        )
        .unwrap();
        lines.tree.as_mut().unwrap().checked = None; // as if RECHECK had passed
        let sent = String::from_utf8(lines.poll().unwrap()).unwrap();
        assert!(
            sent.contains("claude-code:c"),
            "the new child was blocked: {sent}"
        );
    }

    /// R15: a file that was reported and then deleted is forgotten, so a new
    /// one of that name that can't be read is reported too.
    #[test]
    fn a_deleted_file_is_forgotten() {
        let dir = tempfile::tempdir().unwrap();
        let z = dir.path().join("z.jsonl");
        let mut lines = Lines::new(dir.path(), None);
        // Reported as unreadable earlier, and deleted since.
        lines.unreadable.insert(z.clone());
        let mut failed = Vec::new();
        let _ = read_batches(
            std::slice::from_ref(&z),
            &lines.offsets,
            &mut lines.unreadable,
            &mut failed,
        );
        assert!(!lines.unreadable.contains(&z));
    }

    fn under(session: &str, parent: &str, n: u32) -> String {
        format!(
            r#"{{"v":1,"id":"01K00000000000000000000U{n:02}","ts":"2026-09-25T10:00:{n:02}.000Z","type":"session.started","node":"{session}","parent":"{parent}","data":{{}}}}"#
        ) + "\n"
    }

    /// R13: starting to share a tree of many sessions works it out once,
    /// not once per file (each is a full reduce of the log).
    #[test]
    fn a_tree_is_worked_out_once_per_poll() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("claude-code-p.jsonl"),
            start("claude-code:p", "/a"),
        )
        .unwrap();
        for n in 0..5 {
            let child = format!("claude-code:c{n}");
            std::fs::write(
                dir.path().join(format!("claude-code-c{n}.jsonl")),
                under(&child, "claude-code:p", n),
            )
            .unwrap();
        }
        let mut lines = Lines::new(dir.path(), Some("claude-code:p".into()));
        let text = String::from_utf8(lines.poll().unwrap()).unwrap();
        assert_eq!(text.lines().count(), 6, "the root and its five children");
        assert_eq!(lines.tree.as_ref().unwrap().rechecks, 1);
    }

    /// R13, R15: while a member can't be read, it may have restarted under
    /// another parent, taking the sessions under it along: only the root's
    /// lines are sent until it can be read and the tree checked.
    #[cfg(unix)]
    #[test]
    fn a_restarted_member_is_held_back_until_the_tree_can_be_checked() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let file = |name: &str| dir.path().join(name);
        std::fs::write(file("claude-code-p.jsonl"), start("claude-code:p", "/a")).unwrap();
        std::fs::write(
            file("claude-code-c.jsonl"),
            under("claude-code:c", "claude-code:p", 1),
        )
        .unwrap();
        std::fs::write(
            file("claude-code-d.jsonl"),
            under("claude-code:d", "claude-code:c", 2),
        )
        .unwrap();
        let mut lines = Lines::new(dir.path(), Some("claude-code:p".into()));
        let count = |b: Vec<u8>| String::from_utf8(b).unwrap().lines().count();
        assert_eq!(count(lines.poll().unwrap()), 3);

        // c restarts under another session, but can't be read; a session
        // under it carries on; the root does too.
        crate::store::append(
            &file("claude-code-c.jsonl"),
            under("claude-code:c", "claude-code:q", 3).as_bytes(),
        )
        .unwrap();
        std::fs::set_permissions(
            file("claude-code-c.jsonl"),
            std::fs::Permissions::from_mode(0o200),
        )
        .unwrap();
        if std::fs::read(file("claude-code-c.jsonl")).is_ok() {
            return; // running as root: nothing is unreadable
        }
        let status = |node: &str, n: u32| {
            format!(
                r#"{{"v":1,"id":"01K00000000000000000000W{n:02}","ts":"2026-09-25T10:00:{n:02}.000Z","type":"status","node":"{node}","data":{{"state":"working"}}}}"#
            ) + "\n"
        };
        crate::store::append(
            &file("claude-code-d.jsonl"),
            status("claude-code:d", 4).as_bytes(),
        )
        .unwrap();
        crate::store::append(
            &file("claude-code-p.jsonl"),
            status("claude-code:p", 5).as_bytes(),
        )
        .unwrap();
        let sent = String::from_utf8(lines.poll().unwrap()).unwrap();
        assert!(
            !sent.contains("claude-code:d"),
            "sent before it could be checked: {sent}"
        );
        assert!(
            sent.contains("claude-code:p"),
            "the root carries on: {sent}"
        );

        // Readable again: c has left, taking d along.
        std::fs::set_permissions(
            file("claude-code-c.jsonl"),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        let sent = String::from_utf8(lines.poll().unwrap()).unwrap();
        assert!(sent.is_empty(), "they left, so none of it is sent: {sent}");
    }

    /// A status line for `node`.
    fn status(node: &str, n: u32) -> String {
        format!(
            r#"{{"v":1,"id":"01K00000000000000000000W{n:02}","ts":"2026-09-25T10:00:{n:02}.000Z","type":"status","node":"{node}","data":{{"state":"working"}}}}"#
        ) + "\n"
    }

    /// Makes `path` unreadable; false if it still can be (running as root).
    #[cfg(unix)]
    fn make_unreadable(path: &Path) -> bool {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o200)).unwrap();
        std::fs::read(path).is_err()
    }

    #[cfg(unix)]
    fn make_readable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }

    /// R15: a member that can't be read drops out at the next recheck, and
    /// the sessions beside it carry on (they aren't held back for good).
    #[cfg(unix)]
    #[test]
    fn an_unreadable_member_drops_out_and_the_rest_carry_on() {
        let dir = tempfile::tempdir().unwrap();
        let file = |name: &str| dir.path().join(name);
        std::fs::write(file("claude-code-p.jsonl"), start("claude-code:p", "/a")).unwrap();
        std::fs::write(
            file("claude-code-c.jsonl"),
            under("claude-code:c", "claude-code:p", 1),
        )
        .unwrap();
        std::fs::write(
            file("claude-code-d.jsonl"),
            under("claude-code:d", "claude-code:p", 2),
        )
        .unwrap();
        let mut lines = Lines::new(dir.path(), Some("claude-code:p".into()));
        assert_eq!(
            String::from_utf8(lines.poll().unwrap())
                .unwrap()
                .lines()
                .count(),
            3
        );

        // c has more to say (it may have restarted elsewhere), but can't be
        // read; d carries on.
        crate::store::append(
            &file("claude-code-c.jsonl"),
            status("claude-code:c", 3).as_bytes(),
        )
        .unwrap();
        if !make_unreadable(&file("claude-code-c.jsonl")) {
            return;
        }
        crate::store::append(
            &file("claude-code-d.jsonl"),
            status("claude-code:d", 4).as_bytes(),
        )
        .unwrap();
        lines.tree.as_mut().unwrap().checked = None; // as if RECHECK had passed
        let sent = String::from_utf8(lines.poll().unwrap()).unwrap();
        make_readable(&file("claude-code-c.jsonl"));
        assert!(
            sent.contains("claude-code:d"),
            "the sibling was held back: {sent}"
        );
        let tree = lines.tree.as_ref().unwrap();
        assert!(!tree.files.contains(&file("claude-code-c.jsonl")));
    }

    /// R15: a member left out because it couldn't be read rejoins once it
    /// can be, though its size hasn't changed.
    #[cfg(unix)]
    #[test]
    fn a_member_that_can_be_read_again_rejoins() {
        let dir = tempfile::tempdir().unwrap();
        let file = |name: &str| dir.path().join(name);
        std::fs::write(file("claude-code-p.jsonl"), start("claude-code:p", "/a")).unwrap();
        std::fs::write(
            file("claude-code-c.jsonl"),
            under("claude-code:c", "claude-code:p", 1),
        )
        .unwrap();
        let mut lines = Lines::new(dir.path(), Some("claude-code:p".into()));
        assert_eq!(
            String::from_utf8(lines.poll().unwrap())
                .unwrap()
                .lines()
                .count(),
            2
        );

        crate::store::append(
            &file("claude-code-c.jsonl"),
            status("claude-code:c", 2).as_bytes(),
        )
        .unwrap();
        if !make_unreadable(&file("claude-code-c.jsonl")) {
            return;
        }
        // Left out, and looked at again while it still can't be read.
        for _ in 0..2 {
            lines.tree.as_mut().unwrap().checked = None;
            let sent = String::from_utf8(lines.poll().unwrap()).unwrap();
            assert!(!sent.contains("claude-code:c"), "{sent}");
        }
        make_readable(&file("claude-code-c.jsonl"));
        lines.tree.as_mut().unwrap().checked = None;
        let sent = String::from_utf8(lines.poll().unwrap()).unwrap();
        assert!(
            sent.contains(r#""type":"status","node":"claude-code:c""#),
            "it didn't come back: {sent}"
        );
    }

    /// R15: a session under one that can't be read drops out with it, since
    /// the one above may have moved, taking it along.
    #[cfg(unix)]
    #[test]
    fn a_session_under_an_unreadable_one_drops_out_too() {
        let dir = tempfile::tempdir().unwrap();
        let file = |name: &str| dir.path().join(name);
        // p's own log puts m under it (so the link is still known with m's
        // log unreadable); c's puts c under m.
        let returned = r#"{"v":1,"id":"01K00000000000000000000R01","ts":"2026-09-25T10:00:01.000Z","type":"spawn.returned","node":"claude-code:p","data":{"call_id":"x","child":"claude-code:m"}}"#;
        std::fs::write(
            file("claude-code-p.jsonl"),
            start("claude-code:p", "/a") + returned + "\n",
        )
        .unwrap();
        std::fs::write(file("claude-code-m.jsonl"), status("claude-code:m", 2)).unwrap();
        std::fs::write(
            file("claude-code-c.jsonl"),
            under("claude-code:c", "claude-code:m", 3),
        )
        .unwrap();
        let mut lines = Lines::new(dir.path(), Some("claude-code:p".into()));
        let sent = String::from_utf8(lines.poll().unwrap()).unwrap();
        assert!(sent.contains(r#""node":"claude-code:c""#), "{sent}");

        // m has more to say (it may have restarted elsewhere), but can't be
        // read; c carries on.
        crate::store::append(
            &file("claude-code-m.jsonl"),
            under("claude-code:m", "claude-code:q", 5).as_bytes(),
        )
        .unwrap();
        if !make_unreadable(&file("claude-code-m.jsonl")) {
            return;
        }
        crate::store::append(
            &file("claude-code-c.jsonl"),
            status("claude-code:c", 4).as_bytes(),
        )
        .unwrap();
        lines.tree.as_mut().unwrap().checked = None;
        let sent = String::from_utf8(lines.poll().unwrap()).unwrap();
        make_readable(&file("claude-code-m.jsonl"));
        assert!(
            !sent.contains("claude-code:c"),
            "sent though it may have moved: {sent}"
        );
    }

    /// R15: a file the recheck couldn't read, and then could, is forgotten,
    /// so it's reported again if it breaks again.
    #[test]
    fn a_file_the_recheck_can_read_again_is_forgotten() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("claude-code-p.jsonl"),
            start("claude-code:p", "/a"),
        )
        .unwrap();
        let z = dir.path().join("z.jsonl");
        std::fs::create_dir(&z).unwrap();
        let mut lines = Lines::new(dir.path(), Some("claude-code:p".into()));
        let _ = lines.poll().unwrap();
        assert!(lines.unreadable.contains(&z));

        std::fs::remove_dir(&z).unwrap();
        std::fs::write(&z, status("claude-code:z", 1)).unwrap();
        lines.tree.as_mut().unwrap().checked = None;
        let _ = lines.poll().unwrap();
        assert!(!lines.unreadable.contains(&z));
    }

    /// R15: with a file that can't be read, only a session's whole id is
    /// certain: a prefix may match a session in that file too.
    #[test]
    fn a_prefix_is_refused_while_a_file_cant_be_read() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("y.jsonl"), start("claude-code:bar", "/b")).unwrap();
        std::fs::create_dir(dir.path().join("z.jsonl")).unwrap();
        let err = pick_session(dir.path(), "bar").unwrap_err();
        assert!(err.contains("z.jsonl"), "{err}");
        // The whole id of a session whose file can't be read: the file is
        // the way out, not the id.
        let err = pick_session(dir.path(), "claude-code:gone").unwrap_err();
        assert!(err.contains("make it readable"), "{err}");
        assert_eq!(
            pick_session(dir.path(), "claude-code:bar").unwrap().0,
            "claude-code:bar"
        );
    }

    /// R13: an id without a provider isn't one the hooks write; sharing it
    /// would only ever send an empty log.
    #[test]
    fn a_session_id_without_a_provider_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("x.jsonl"), start("foo", "/a")).unwrap();
        assert!(pick_session(dir.path(), "foo").is_err());
        std::fs::write(dir.path().join("y.jsonl"), start("claude-code:bar", "/b")).unwrap();
        assert_eq!(
            pick_session(dir.path(), "bar").unwrap().0,
            "claude-code:bar"
        );
    }

    #[test]
    fn chunks_end_at_line_boundaries() {
        let buf = b"aaaa\nbbbb\ncccc\n";
        assert_eq!(chunk_len(buf, 100), buf.len());
        assert_eq!(chunk_len(buf, 12), 10, "two whole lines fit");
        assert_eq!(chunk_len(buf, 3), 5, "one overlong line goes alone");
    }

    #[test]
    fn base64url_matches_the_standard() {
        assert_eq!(base64url(b""), "");
        assert_eq!(base64url(b"f"), "Zg");
        assert_eq!(base64url(b"fo"), "Zm8");
        assert_eq!(base64url(b"foo"), "Zm9v");
        assert_eq!(base64url("pässwörd?>".as_bytes()), "cMOkc3N3w7ZyZD8-");
    }

    #[test]
    fn default_password_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("remote.json");
        assert_eq!(load_default_password(&path).unwrap(), None);
        save_default_password(&path, "s3cret").unwrap();
        assert_eq!(
            load_default_password(&path).unwrap().as_deref(),
            Some("s3cret")
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        clear_default_password(&path).unwrap();
        assert_eq!(load_default_password(&path).unwrap(), None);
        clear_default_password(&path).unwrap();
    }

    #[test]
    fn lines_returns_only_new_whole_lines() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("x.jsonl");
        let mut lines = Lines::new(dir.path(), None);
        crate::store::append(&file, b"one\ntw").unwrap();
        assert_eq!(lines.poll().unwrap(), b"one\n");
        crate::store::append(&file, b"o\n").unwrap();
        assert_eq!(lines.poll().unwrap(), b"two\n");
        assert!(lines.poll().unwrap().is_empty());
        fs::write(dir.path().join("notes.txt"), "ignored").unwrap();
        assert!(lines.poll().unwrap().is_empty());
    }

    /// R15: a file that can't be read is skipped (and reported), not a
    /// reason to share nothing; once it can be read, it's shared too, and
    /// nothing from the others is lost meanwhile.
    #[test]
    fn a_file_that_cant_be_read_doesnt_stop_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.jsonl");
        crate::store::append(&a, b"a1\n").unwrap();
        // Something that can't be read as a file.
        let z = dir.path().join("z.jsonl");
        fs::create_dir(&z).unwrap();
        let mut lines = Lines::new(dir.path(), None);
        assert_eq!(lines.poll().unwrap(), b"a1\n");

        crate::store::append(&a, b"a2\n").unwrap();
        assert_eq!(lines.poll().unwrap(), b"a2\n");

        fs::remove_dir(&z).unwrap();
        crate::store::append(&z, b"z1\n").unwrap();
        assert_eq!(lines.poll().unwrap(), b"z1\n");
        assert!(lines.poll().unwrap().is_empty());
    }
}
