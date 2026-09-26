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
//!    already there. Retrying a chunk rewrites the same document.
//!
//! Bodies are raw JSON Lines, never wrapped in JSON, and are cut only at line
//! boundaries.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

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
    let mut pending = source
        .poll()
        .map_err(|e| format!("reading {}: {e}", events.display()))?;

    let client = Client::new(&base);
    let first = chunk_len(&pending, MAX_CHUNK);
    let log = client
        .create(&pending[..first], password.as_deref())
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

    let mut sent = first as u64;
    pending.drain(..first);
    let mut backoff = Duration::ZERO;
    loop {
        while !pending.is_empty() {
            let n = chunk_len(&pending, MAX_CHUNK);
            match client.append(&log, sent, &pending[..n]) {
                Ok(()) => {
                    sent += n as u64;
                    pending.drain(..n);
                    if !backoff.is_zero() {
                        eprintln!("Reconnected; caught up.");
                        backoff = Duration::ZERO;
                    }
                }
                Err(SendError::Fatal(e)) => return Err(e),
                Err(SendError::Retry(e)) => {
                    if backoff.is_zero() {
                        eprintln!("Couldn't reach {base}: {e}. Keeping new events and retrying…");
                    }
                    backoff = (backoff * 2).clamp(Duration::from_secs(2), MAX_BACKOFF);
                    std::thread::sleep(backoff);
                    break;
                }
            }
        }
        std::thread::sleep(POLL);
        // A read error (say, the directory is briefly missing) is retried.
        if let Ok(new) = source.poll() {
            pending.extend(new);
        }
    }
}

// ---------- reading new lines ----------

/// Tracks how far each events file has been read and returns only complete
/// new lines, as raw bytes.
pub struct Lines {
    dir: PathBuf,
    /// Sharing one session: only the files of its tree.
    tree: Option<Tree>,
    offsets: BTreeMap<PathBuf, u64>,
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
    /// How many times the tree has been worked out (each a full reduce).
    rechecks: u32,
}

impl Tree {
    /// Notes whether any file outside the tree has changed.
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
    }

    /// Works out the tree's files again. Returns whether it could: if the
    /// log can't be read just now, it stays stale, to try again.
    fn recheck(&mut self, dir: &Path) -> bool {
        self.checked = Some(Instant::now());
        self.rechecks += 1;
        match tree_files(dir, &self.root) {
            Ok(files) => {
                self.files = files;
                self.stale = false;
                self.failing = false;
                true
            }
            Err(e) => {
                if !self.failing {
                    eprintln!(
                        "Can't read the events to see which sessions belong to this one ({e}); \
                         holding back any that may have moved, and trying again."
                    );
                    self.failing = true;
                }
                false
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
                rechecks: 0,
            }
        });
        Lines {
            dir: dir.to_path_buf(),
            tree,
            offsets: BTreeMap::new(),
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
        for entry in entries {
            let path = entry?.path();
            if path.extension().is_some_and(|e| e == "jsonl") {
                paths.push(path);
            }
        }
        let Some(tree) = &mut self.tree else {
            let batches = read_batches(&paths, &self.offsets)?;
            return Ok(self.commit(batches));
        };
        tree.notice(&paths);

        let members: Vec<PathBuf> = paths
            .iter()
            .filter(|p| tree.files.contains(*p))
            .cloned()
            .collect();
        let mut batches = read_batches(&members, &self.offsets)?;
        let restarted = batches.iter().any(|b| {
            b.path != tree.root_file && contains(&b.lines, br#""type":"session.started""#)
        });
        let due = tree.checked.is_none_or(|t| t.elapsed() >= RECHECK);
        // A restart is checked straight away, unless the log has been
        // unreadable, when it waits like any other recheck.
        let recheck_now = (restarted && !tree.failing) || ((restarted || tree.stale) && due);
        let checked = recheck_now && tree.recheck(&self.dir);
        if checked {
            // Files that just joined: read them now, after the recheck.
            let joined: Vec<PathBuf> = paths
                .iter()
                .filter(|p| tree.files.contains(*p) && !members.contains(p))
                .cloned()
                .collect();
            batches.extend(read_batches(&joined, &self.offsets)?);
        } else if restarted {
            // Can't tell whether a restarted session left, taking the
            // sessions under it along: hold back all but the root's lines
            // until we can.
            batches.retain(|b| b.path == tree.root_file);
        }
        let files = tree.files.clone();
        batches.retain(|b| files.contains(&b.path));
        Ok(self.commit(batches))
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

/// New whole lines in each of `paths`, past its offset.
fn read_batches(paths: &[PathBuf], offsets: &BTreeMap<PathBuf, u64>) -> io::Result<Vec<Batch>> {
    let mut batches = Vec::new();
    for path in paths {
        let size = fs::metadata(path)?.len();
        let mut offset = offsets.get(path).copied().unwrap_or(0);
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
        batches.push(Batch {
            path: path.clone(),
            lines,
            offset,
        });
    }
    Ok(batches)
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

/// The files of every session (and `run`) in `root`'s tree.
fn tree_files(dir: &Path, root: &str) -> io::Result<BTreeSet<PathBuf>> {
    let loaded = crate::store::load_events(dir)?;
    let graph = crate::reducer::reduce(loaded.events, &crate::reducer::Options::default());
    let mut files = BTreeSet::from([session_path(dir, root)]);
    let mut seen = BTreeSet::new();
    let mut queue = vec![root.to_string()];
    while let Some(id) = queue.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        let session = id.split('/').next().unwrap_or(&id);
        files.insert(session_path(dir, session));
        if let Some(node) = graph.nodes.get(&id) {
            queue.extend(node.children.iter().cloned());
        }
    }
    Ok(files)
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
}

impl SendError {
    fn message(self) -> String {
        match self {
            SendError::Retry(m) | SendError::Fatal(m) => m,
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

    fn start(session: &str, cwd: &str) -> String {
        format!(
            r#"{{"v":1,"id":"01K00000000000000000000S01","ts":"2026-09-25T10:00:00.000Z","type":"session.started","node":"{session}","data":{{"cwd":"{cwd}"}}}}"#
        ) + "\n"
    }

    /// R13: a recheck that can't read the log keeps the tree stale, so the
    /// change that prompted it isn't lost.
    #[test]
    fn a_failed_recheck_tries_again() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("claude-code-p.jsonl"),
            start("claude-code:p", "/a"),
        )
        .unwrap();
        // Something load_events can't read: a folder where a file should be.
        std::fs::create_dir(dir.path().join("broken.jsonl")).unwrap();
        let mut lines = Lines::new(dir.path(), Some("claude-code:p".into()));
        let _ = lines.poll();
        assert!(lines.tree.as_ref().unwrap().stale, "gave up on the change");
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

    /// R13: while the log can't be read, a member that restarted (and so
    /// may have left) isn't sent; once it can, it's sent only if it stayed.
    #[test]
    fn a_restarted_member_is_held_back_until_the_tree_can_be_checked() {
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
        assert_eq!(
            String::from_utf8(lines.poll().unwrap())
                .unwrap()
                .lines()
                .count(),
            3
        );

        // It restarts under another session while the log can't be read,
        // and a session under it carries on.
        std::fs::create_dir(file("broken.jsonl")).unwrap();
        crate::store::append(
            &file("claude-code-c.jsonl"),
            under("claude-code:c", "claude-code:q", 2).as_bytes(),
        )
        .unwrap();
        let working = r#"{"v":1,"id":"01K00000000000000000000W01","ts":"2026-09-25T10:00:09.000Z","type":"status","node":"claude-code:d","data":{"state":"working"}}"#;
        crate::store::append(
            &file("claude-code-d.jsonl"),
            format!("{working}\n").as_bytes(),
        )
        .unwrap();
        let rechecks = lines.tree.as_ref().unwrap().rechecks;
        for _ in 0..3 {
            let sent = String::from_utf8(lines.poll().unwrap()).unwrap();
            assert!(sent.is_empty(), "sent before it could be checked: {sent}");
        }
        assert_eq!(
            lines.tree.as_ref().unwrap().rechecks,
            rechecks + 1,
            "while failing, it retries only every RECHECK"
        );

        std::fs::remove_dir(file("broken.jsonl")).unwrap();
        lines.tree.as_mut().unwrap().checked = None; // as if RECHECK had passed
        assert!(
            lines.poll().unwrap().is_empty(),
            "they left, so none of it is sent"
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
}
