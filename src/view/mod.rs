//! `agent-graph view`: a local web UI that updates live and can step back
//! through each session's timeline.
//!
//! One thread polls the events directory; each HTTP connection gets its own
//! thread. Pages learn about new events over Server-Sent Events and then ask
//! for the graph again, so the Rust reducer stays the only implementation of
//! the graph logic.

mod http;
pub mod tail;

use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use http::{Request, respond};
use tail::Tail;

use crate::timeline::{self as api, ApiError, Timed};

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

/// Loads the events, then serves `listener` and watches for new events on
/// background threads.
pub fn start(
    events_dir: &Path,
    listener: TcpListener,
    stale_after: Duration,
) -> Result<(), String> {
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let shared = Arc::new(Shared {
        tail: Mutex::new(Tail::new(events_dir)),
        version: Mutex::new(0),
        changed: Condvar::new(),
        stale_after,
        port,
        events_dir: events_dir.to_path_buf(),
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
    if !host_allowed(req.host.as_deref(), shared.port) {
        return respond(stream, 403, "text/plain", &[], b"Forbidden");
    }
    if req.method != "GET" {
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
}
