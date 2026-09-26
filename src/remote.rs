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

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub const DEFAULT_URL: &str = "https://agentgraph.chofter.com";
/// The most sent in one request; the site rejects bigger bodies.
pub const MAX_CHUNK: usize = 256 * 1024;
/// How often to look for new lines.
const POLL: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(60);

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
            let (file, what) = session_file(&events, want)?;
            (Some(file), what)
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
    only: Option<PathBuf>,
    offsets: BTreeMap<PathBuf, u64>,
}

impl Lines {
    pub fn new(dir: &Path, only: Option<PathBuf>) -> Lines {
        Lines {
            dir: dir.to_path_buf(),
            only,
            offsets: BTreeMap::new(),
        }
    }

    pub fn poll(&mut self) -> io::Result<Vec<u8>> {
        let mut out = Vec::new();
        let entries = match fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(e),
        };
        for entry in entries {
            let path = entry?.path();
            if path.extension().is_none_or(|e| e != "jsonl")
                || self.only.as_ref().is_some_and(|o| o != &path)
            {
                continue;
            }
            let size = fs::metadata(&path)?.len();
            let mut offset = self.offsets.get(&path).copied().unwrap_or(0);
            // A file that shrank was rewritten; send it again. The site drops
            // events it has already seen.
            if size < offset {
                offset = 0;
            }
            if size > offset {
                let mut file = File::open(&path)?;
                file.seek(SeekFrom::Start(offset))?;
                let mut buf = Vec::new();
                file.take(size - offset).read_to_end(&mut buf)?;
                // Only whole lines; the rest waits for the next poll.
                let end = buf.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
                out.extend_from_slice(&buf[..end]);
                offset += end as u64;
            }
            self.offsets.insert(path, offset);
        }
        Ok(out)
    }
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

/// The events file for the session matching `want`, and how to describe it.
/// "current" must be the session this runs in: anything looser (the newest
/// session, say) could publish another project's.
fn session_file(events: &Path, want: &str) -> Result<(PathBuf, String), String> {
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
    let (provider, id) = session.split_once(':').ok_or("unexpected node id")?;
    let folder = graph.nodes.get(&session).and_then(|n| n.cwd.clone());
    // Both come from the log: cleaned, like everything else printed from it.
    let clean = crate::render::clean;
    let what = match folder {
        Some(folder) => format!("session {} (in {})", clean(&session), clean(&folder)),
        None => format!("session {}", clean(&session)),
    };
    Ok((
        events.join(format!("{}.jsonl", crate::paths::file_key(provider, id))),
        what,
    ))
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
