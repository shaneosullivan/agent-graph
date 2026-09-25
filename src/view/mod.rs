//! `agent-graph view`: a local web UI that updates live and can step back
//! through each session's timeline.
//!
//! One thread polls the events directory; each HTTP connection gets its own
//! thread. Pages learn about new events over Server-Sent Events and then ask
//! for the graph again, so the Rust reducer stays the only implementation of
//! the graph logic.
//!
//! Being on the computer the sessions ran on, it can also reopen one in its
//! agent (`POST /api/open`, see `open`).

mod http;
pub mod open;
pub mod tail;

use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use http::{Request, respond};
use tail::Tail;

use crate::event::Payload;
use crate::resume::{self, Resume};
use crate::timeline::{self as api, ApiError, Environment, Timed};

/// How often the events directory is checked for new lines.
const POLL: Duration = Duration::from_millis(250);
/// How often an idle event stream sends a keep-alive comment.
const HEARTBEAT: Duration = Duration::from_secs(15);

const INDEX_HTML: &str = include_str!("assets/index.html");
const APP_CSS: &str = include_str!("assets/app.css");
const APP_JS: &str = include_str!("assets/app.js");

/// Everything the page needs comes from this server; nothing may be loaded
/// or sent anywhere else.
const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; \
                   img-src 'self' data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

pub struct Options {
    pub port: u16,
    pub open: bool,
    pub stale_after: Duration,
}

pub fn run(events_dir: &Path, opts: Options) -> Result<(), String> {
    let listener = TcpListener::bind(("127.0.0.1", opts.port)).map_err(|e| {
        format!(
            "can't listen on port {}: {e}. Is the viewer already running? Try --port.",
            opts.port
        )
    })?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let url = format!("http://localhost:{port}/");
    start(events_dir, listener, opts.stale_after)?;
    println!("Agent Graph viewer: {url}");
    println!("Reading events from {}", events_dir.display());
    println!("Press Ctrl+C to stop.");
    if opts.open {
        open_browser(&url);
    }
    loop {
        thread::park();
    }
}

/// Runs a command that reopens a session: `open::in_terminal`, or a stand-in
/// in tests.
pub type Launch = fn(&Resume) -> Result<(), String>;

/// Loads the events, then serves `listener` and watches for new events on
/// background threads.
pub fn start(
    events_dir: &Path,
    listener: TcpListener,
    stale_after: Duration,
) -> Result<(), String> {
    start_with(events_dir, listener, stale_after, open::in_terminal)
}

/// `start`, opening sessions with `launch`.
pub fn start_with(
    events_dir: &Path,
    listener: TcpListener,
    stale_after: Duration,
    launch: Launch,
) -> Result<(), String> {
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let shared = Arc::new(Shared {
        tail: Mutex::new(Tail::new(events_dir)),
        version: Mutex::new(0),
        changed: Condvar::new(),
        stale_after,
        port,
        events_dir: events_dir.to_path_buf(),
        launch,
    });
    shared
        .poll()
        .map_err(|e| format!("reading {}: {e}", events_dir.display()))?;

    let watcher = Arc::clone(&shared);
    thread::spawn(move || {
        loop {
            thread::sleep(POLL);
            // A read error (e.g. the directory is briefly unavailable) is
            // retried on the next poll.
            let _ = watcher.poll();
        }
    });

    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let shared = Arc::clone(&shared);
            thread::spawn(move || handle(stream, &shared));
        }
    });
    Ok(())
}

struct Shared {
    tail: Mutex<Tail>,
    /// Bumped whenever the events change; streams wait on `changed`.
    version: Mutex<u64>,
    changed: Condvar,
    stale_after: Duration,
    port: u16,
    events_dir: PathBuf,
    launch: Launch,
}

impl Shared {
    fn poll(&self) -> std::io::Result<()> {
        let changed = self.tail.lock().expect("tail lock").poll()?;
        if changed {
            *self.version.lock().expect("version lock") += 1;
            self.changed.notify_all();
        }
        Ok(())
    }

    /// Waits until the version moves past `seen`, or `timeout` passes.
    fn wait(&self, seen: u64, timeout: Duration) -> u64 {
        let version = self.version.lock().expect("version lock");
        let (version, _) = self
            .changed
            .wait_timeout_while(version, timeout, |v| *v == seen)
            .expect("version lock");
        *version
    }
}

fn handle(mut stream: TcpStream, shared: &Shared) {
    let Ok(req) = http::read_request(&mut stream) else {
        return;
    };
    let _ = route(&mut stream, &req, shared);
}

fn route(stream: &mut TcpStream, req: &Request, shared: &Shared) -> std::io::Result<()> {
    // Only answer requests addressed to this machine by name. This stops a web
    // page from reaching the viewer through DNS rebinding.
    if !host_allowed(req.header("host"), shared.port) {
        return respond(stream, 403, "text/plain", &[], b"Forbidden");
    }
    if req.method == "POST" {
        // Other websites can send requests to localhost too. Browsers say
        // where a POST comes from, so only this viewer's own page gets here.
        if !same_origin(req.header("origin"), req.header("host")) {
            return respond(stream, 403, "text/plain", &[], b"Forbidden");
        }
        return match req.path.as_str() {
            "/api/open" => open_session(stream, req, shared),
            _ => respond(stream, 404, "text/plain", &[], b"Not found"),
        };
    }
    if req.method != "GET" || req.path == "/api/open" {
        return respond(stream, 405, "text/plain", &[], b"Method not allowed");
    }
    let json = |stream: &mut TcpStream, result: Result<String, ApiError>| match result {
        Ok(body) => respond(stream, 200, "application/json", &[], body.as_bytes()),
        Err(ApiError::NotFound(msg)) => respond(stream, 404, "text/plain", &[], msg.as_bytes()),
        Err(ApiError::Failed(msg)) => respond(stream, 500, "text/plain", &[], msg.as_bytes()),
    };
    match req.path.as_str() {
        "/" | "/index.html" => respond(
            stream,
            200,
            "text/html; charset=utf-8",
            &[("Content-Security-Policy", CSP)],
            INDEX_HTML.as_bytes(),
        ),
        "/app.css" => respond(
            stream,
            200,
            "text/css; charset=utf-8",
            &[],
            APP_CSS.as_bytes(),
        ),
        "/app.js" => respond(
            stream,
            200,
            "text/javascript; charset=utf-8",
            &[],
            APP_JS.as_bytes(),
        ),
        "/api/graph" => {
            let result = {
                let tail = shared.tail.lock().expect("tail lock");
                api::graph(
                    &tail.events,
                    req.param("until"),
                    crate::clock::now(),
                    shared.stale_after,
                    Environment::Local,
                )
            };
            json(stream, result)
        }
        "/api/timeline" => {
            let Some(root) = req.param("root") else {
                return respond(stream, 404, "text/plain", &[], b"missing root");
            };
            let result = {
                let tail = shared.tail.lock().expect("tail lock");
                api::timeline(&tail.events, root, crate::clock::now(), shared.stale_after)
            };
            json(stream, result)
        }
        "/api/image.png" | "/api/image.svg" => {
            let Some(root) = req.param("root") else {
                return respond(stream, 404, "text/plain", &[], b"missing root");
            };
            let svg = req.path.ends_with(".svg");
            let theme = match req.param("theme") {
                Some("dark") => crate::image::Theme::Dark,
                _ => crate::image::Theme::Light,
            };
            // Render outside the lock; only the events are needed from it.
            let events = shared.tail.lock().expect("tail lock").events.clone();
            let (kind, disposition) = if svg {
                ("image/svg+xml", "attachment; filename=\"agent-graph.svg\"")
            } else {
                ("image/png", "attachment; filename=\"agent-graph.png\"")
            };
            match image(
                &events,
                root,
                req.param("until"),
                theme,
                svg,
                shared.stale_after,
            ) {
                Ok(bytes) => respond(
                    stream,
                    200,
                    kind,
                    &[("Content-Disposition", disposition)],
                    &bytes,
                ),
                Err(ApiError::NotFound(msg)) => {
                    respond(stream, 404, "text/plain", &[], msg.as_bytes())
                }
                Err(ApiError::Failed(msg)) => {
                    respond(stream, 500, "text/plain", &[], msg.as_bytes())
                }
            }
        }
        "/api/info" => {
            let body = serde_json::json!({
                "events_dir": shared.events_dir.display().to_string(),
                "version": *shared.version.lock().expect("version lock"),
                // The page measures "5m ago" against this, so a pinned clock
                // (AGENT_GRAPH_NOW) applies there too.
                "now_ms": crate::clock::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or_default(),
            });
            respond(
                stream,
                200,
                "application/json",
                &[],
                body.to_string().as_bytes(),
            )
        }
        "/api/stream" => stream_changes(stream, shared),
        _ => respond(stream, 404, "text/plain", &[], b"Not found"),
    }
}

/// `POST /api/open?node=<id>`: reopens a session in its agent, in a new
/// terminal window. Only the node id comes from the page; the command is
/// worked out from the log. Replies `{"ok": true, "command": …}`, or
/// `{"error": …, "command": …}` with the command to run yourself.
fn open_session(stream: &mut TcpStream, req: &Request, shared: &Shared) -> std::io::Result<()> {
    let reply = |stream: &mut TcpStream, status: u16, body: serde_json::Value| {
        respond(
            stream,
            status,
            "application/json",
            &[],
            body.to_string().as_bytes(),
        )
    };
    let id = req.param("node").unwrap_or_default();
    let (found, transcript) = {
        let tail = shared.tail.lock().expect("tail lock");
        let found = api::graph_at(&tail.events, None, crate::clock::now(), shared.stale_after)
            .ok()
            .and_then(|(graph, _, _)| graph.nodes.get(id).and_then(resume::resume));
        (found, transcript_of(&tail.events, id))
    };
    let Some(r) = found else {
        let error = "That can't be opened: only sessions whose agent can resume them can.";
        return reply(stream, 404, serde_json::json!({ "error": error }));
    };
    let command = open::command_line(&r);
    let problem = if !Path::new(&r.cwd).is_dir() {
        Some(format!("Its folder, {}, isn't on this computer.", r.cwd))
    } else {
        transcript
            .filter(|t| !Path::new(t).is_file())
            .map(|t| format!("{} doesn't have it any more: {t} is gone.", r.app))
    };
    if let Some(error) = problem {
        return reply(
            stream,
            404,
            serde_json::json!({ "error": error, "command": command }),
        );
    }
    match (shared.launch)(&r) {
        Ok(()) => reply(
            stream,
            200,
            serde_json::json!({ "ok": true, "command": command }),
        ),
        Err(e) => reply(
            stream,
            500,
            serde_json::json!({ "error": format!("Couldn't open a terminal: {e}."), "command": command }),
        ),
    }
}

/// Where the agent keeps a session's conversation, from its latest start.
fn transcript_of(events: &[Timed], id: &str) -> Option<String> {
    events
        .iter()
        .rev()
        .filter(|t| t.event.node == id && t.event.kind == "session.started")
        .find_map(|t| match t.event.payload() {
            Payload::SessionStarted(d) => d.transcript_path,
            _ => None,
        })
}

/// Sends `changed` whenever new events arrive, until the page goes away.
fn stream_changes(stream: &mut TcpStream, shared: &Shared) -> std::io::Result<()> {
    use std::io::Write;
    http::start_event_stream(stream)?;
    let mut seen = *shared.version.lock().expect("version lock");
    loop {
        let version = shared.wait(seen, HEARTBEAT);
        if version == seen {
            stream.write_all(b": ping\n\n")?;
        } else {
            seen = version;
            stream.write_all(format!("event: changed\ndata: {version}\n\n").as_bytes())?;
        }
        stream.flush()?;
    }
}

fn host_allowed(host: Option<&str>, port: u16) -> bool {
    let Some(host) = host else { return false };
    ["localhost", "127.0.0.1", "[::1]"]
        .iter()
        .any(|name| host.eq_ignore_ascii_case(&format!("{name}:{port}")))
}

/// Whether a request's `Origin` is this viewer's own page. `host` has already
/// been checked; a missing `Origin` (or "null") doesn't count.
fn same_origin(origin: Option<&str>, host: Option<&str>) -> bool {
    match (origin, host) {
        (Some(origin), Some(host)) => origin.eq_ignore_ascii_case(&format!("http://{host}")),
        _ => false,
    }
}

fn open_browser(url: &str) {
    use std::process::{Command, Stdio};
    let mut cmd = if cfg!(target_os = "macos") {
        Command::new("open")
    } else if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/C", "start", ""]);
        c
    } else {
        Command::new("xdg-open")
    };
    let _ = cmd
        .arg(url)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

/// A PNG (or SVG) of the tree under `root`, as of `until` or now.
fn image(
    events: &[Timed],
    root: &str,
    until: Option<&str>,
    theme: crate::image::Theme,
    svg_only: bool,
    stale_after: Duration,
) -> Result<Vec<u8>, ApiError> {
    let (graph, _, now) = api::graph_at(events, until, crate::clock::now(), stale_after)?;
    if !graph.nodes.contains_key(root) {
        return Err(ApiError::NotFound(format!("no node {root} at that point")));
    }
    let svg = crate::image::svg(
        &graph,
        &[root.to_string()],
        &crate::image::Options { theme, as_of: now },
    );
    if svg_only {
        return Ok(svg.into_bytes());
    }
    crate::image::png(&svg).map_err(ApiError::Failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_local_host_names_are_allowed() {
        assert!(host_allowed(Some("localhost:7777"), 7777));
        assert!(host_allowed(Some("127.0.0.1:7777"), 7777));
        assert!(host_allowed(Some("[::1]:7777"), 7777));
        assert!(!host_allowed(Some("localhost:8080"), 7777));
        assert!(!host_allowed(Some("evil.example:7777"), 7777));
        assert!(!host_allowed(None, 7777));
    }

    #[test]
    fn only_the_viewer_itself_may_post() {
        let host = Some("localhost:7777");
        assert!(same_origin(Some("http://localhost:7777"), host));
        assert!(!same_origin(Some("http://127.0.0.1:7777"), host));
        assert!(!same_origin(Some("https://evil.example"), host));
        assert!(!same_origin(Some("null"), host));
        assert!(!same_origin(None, host));
    }
}
