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
//!
//! Listening only on 127.0.0.1 keeps other computers out, but not other
//! accounts on this one. So each run makes a random key, and the link it
//! prints carries it (`/?key=…`). The page (static, and built into the
//! binary) keeps the key in its own storage and sends it with every API
//! request in the `X-Agent-Graph-Key` header, which another website can't
//! send. (Not a cookie: browsers send cookies to every port on localhost,
//! so any other local web server would receive it.) The event stream, which
//! can't set headers, carries it as `?key=`.

pub(crate) mod desktop;
mod http;
mod names;
pub mod open;
mod running;
pub mod tail;

use std::collections::BTreeMap;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, SystemTime};

use desktop::Desktop;
use http::{Request, respond};
use names::Names;
use tail::Tail;

use crate::event::Payload;
use crate::resume::{self, Resume};
use crate::timeline::{self as api, ApiError, Environment, Timed};

/// How often the events directory is checked for new lines.
const POLL: Duration = Duration::from_millis(250);
/// How often sessions' transcripts are checked for a new name (see `names`).
const NAMES_POLL: Duration = Duration::from_secs(1);
/// How often an idle event stream sends a keep-alive comment.
const HEARTBEAT: Duration = Duration::from_secs(15);
/// Most connections served at once (each open page holds one for its event
/// stream); more are closed straight away, so a flood can't use up every
/// thread. (A local user without the key can still lock the viewer out for
/// as long as they keep the connections coming, but not read anything.)
pub const MAX_CONNECTIONS: usize = 256;

const INDEX_HTML: &str = include_str!("assets/index.html");
const APP_CSS: &str = include_str!("assets/app.css");
const APP_JS: &str = include_str!("assets/app.js");
/// D3, for the graph view: loaded by the page only when that's chosen.
const D3_JS: &str = include_str!("assets/d3.min.js");
/// The logo (scripts/make-icons.py): the favicon, and the header's.
const ICON_SVG: &str = include_str!("assets/icon.svg");

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
    let listeners = match bind(opts.port) {
        Ok(listeners) => listeners,
        // A viewer's already running there. It's replaced, since it may be
        // an older version; only a viewer of these events, which takes the
        // key it recorded, is ever stopped (see `running`).
        Err(e) => {
            let old = running::find(events_dir, opts.port).ok_or(e)?;
            let Some(pid) = old.pid else {
                // No process to stop in its record: its link will do.
                let url = link(opts.port, &old.key);
                println!("Agent Graph viewer (already running): {url}");
                if opts.open {
                    open_browser(&url);
                }
                return Ok(());
            };
            println!(
                "Stopping the viewer already running on port {} (process {pid}), to start this one.",
                opts.port
            );
            running::stop(pid, false)?;
            match bind_when_free(opts.port) {
                Ok(listeners) => listeners,
                // Still there: made to stop, this time.
                Err(_) => {
                    running::stop(pid, true)?;
                    bind_when_free(opts.port)?
                }
            }
        }
    };
    let port = listeners[0].local_addr().map_err(|e| e.to_string())?.port();
    let key = start_on(events_dir, listeners, opts.stale_after, open::launch)?;
    // So another `view` on this port can point here (see `running`).
    if let Err(e) = running::record(events_dir, port, &key) {
        eprintln!("Note: couldn't record this viewer's link for others to find: {e}");
    }
    let url = link(port, &key);
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

/// `bind`, retried for a few seconds while the port's freed (by the viewer
/// that was just stopped there).
fn bind_when_free(port: u16) -> Result<Vec<TcpListener>, String> {
    let start = std::time::Instant::now();
    loop {
        match bind(port) {
            Ok(listeners) => return Ok(listeners),
            Err(e) if start.elapsed() > Duration::from_secs(3) => return Err(e),
            Err(_) => thread::sleep(Duration::from_millis(100)),
        }
    }
}

/// Listens on `port` of 127.0.0.1, and of [::1] too when there's IPv6.
///
/// Browsers try `localhost` as [::1] first, so if another program (another
/// account's, say) were listening there, a `localhost` link would go to it.
/// The printed link uses 127.0.0.1 to avoid that, and the viewer refuses to
/// start if [::1] at its port is taken, so typing `localhost` is safe too.
pub fn bind(port: u16) -> Result<Vec<TcpListener>, String> {
    let v4 = TcpListener::bind(("127.0.0.1", port)).map_err(|e| {
        format!("can't listen on port {port}: {e}. Is the viewer already running? Try --port.")
    })?;
    let port = v4.local_addr().map_err(|e| e.to_string())?.port();
    match TcpListener::bind(("::1", port)) {
        Ok(v6) => Ok(vec![v4, v6]),
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => Err(format!(
            "something else is listening on [::1]:{port} (localhost over IPv6), where a \
             browser could send the viewer's link. Try another --port."
        )),
        // No IPv6 here: nothing can be listening there either.
        Err(_) => Ok(vec![v4]),
    }
}

/// The link to the viewer on `port`, carrying `key`. It names 127.0.0.1
/// rather than `localhost`, so it can't be sent anywhere else (see `bind`).
pub fn link(port: u16, key: &str) -> String {
    format!("http://127.0.0.1:{port}/?key={key}")
}

/// Reopens a session: `open::launch`, or a stand-in in tests.
pub type Launch = fn(&Resume) -> Result<(), String>;

/// Loads the events, then serves `listener` and watches for new events on
/// background threads. Returns the key the page's link must carry.
pub fn start(
    events_dir: &Path,
    listener: TcpListener,
    stale_after: Duration,
) -> Result<String, String> {
    start_with(events_dir, listener, stale_after, open::launch)
}

/// `start`, opening sessions with `launch`.
pub fn start_with(
    events_dir: &Path,
    listener: TcpListener,
    stale_after: Duration,
    launch: Launch,
) -> Result<String, String> {
    start_on(events_dir, vec![listener], stale_after, launch)
}

/// `start_with`, serving each of `listeners` (all on the same port).
fn start_on(
    events_dir: &Path,
    listeners: Vec<TcpListener>,
    stale_after: Duration,
    launch: Launch,
) -> Result<String, String> {
    let key = new_key();
    let port = listeners
        .first()
        .ok_or("nothing to listen on")?
        .local_addr()
        .map_err(|e| e.to_string())?
        .port();
    let shared = Arc::new(Shared {
        tail: Mutex::new(Tail::new(events_dir)),
        names: Mutex::new(Names::default()),
        desktop: Mutex::new(Desktop::new(
            desktop::sessions_dir(),
            desktop::chatgpt_installed(),
        )),
        version: Mutex::new(0),
        changed: Condvar::new(),
        stale_after,
        port,
        events_dir: events_dir.to_path_buf(),
        launch,
        key: key.clone(),
        connections: AtomicUsize::new(0),
    });
    shared
        .poll()
        .map_err(|e| format!("reading {}: {e}", events_dir.display()))?;

    shared.poll_names();

    let watcher = Arc::clone(&shared);
    thread::spawn(move || {
        let mut names_at = std::time::Instant::now();
        loop {
            thread::sleep(POLL);
            // A read error (e.g. the directory is briefly unavailable) is
            // retried on the next poll.
            let _ = watcher.poll();
            if names_at.elapsed() >= NAMES_POLL {
                watcher.poll_names();
                names_at = std::time::Instant::now();
            }
        }
    });

    for listener in listeners {
        let shared = Arc::clone(&shared);
        thread::spawn(move || accept(listener, shared));
    }
    Ok(key)
}

/// Serves `listener`'s connections, each on its own thread, up to
/// `MAX_CONNECTIONS` at once across all listeners.
fn accept(listener: TcpListener, shared: Arc<Shared>) {
    for stream in listener.incoming().flatten() {
        // Each connection gets a thread; past the limit, new ones are
        // closed rather than piling up.
        if shared.connections.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
            shared.connections.fetch_sub(1, Ordering::SeqCst);
            continue;
        }
        let shared = Arc::clone(&shared);
        thread::spawn(move || {
            // Gives the slot back however the handler ends, panics included.
            let _slot = Slot(&shared.connections);
            handle(stream, &shared);
        });
    }
}

struct Shared {
    tail: Mutex<Tail>,
    /// Sessions' names, newer than the log's (see `names`).
    names: Mutex<Names>,
    /// The sessions desktop apps have, which open there (see `desktop`).
    desktop: Mutex<Desktop>,
    /// Bumped whenever the events change; streams wait on `changed`.
    version: Mutex<u64>,
    changed: Condvar,
    stale_after: Duration,
    port: u16,
    events_dir: PathBuf,
    launch: Launch,
    /// This run's key; see the module docs.
    key: String,
    /// Connections being served now.
    connections: AtomicUsize,
}

impl Shared {
    fn poll(&self) -> std::io::Result<()> {
        let changed = self.tail.lock().expect("tail lock").poll()?;
        if changed {
            self.bump();
        }
        Ok(())
    }

    /// Re-reads what the agents' own records say: sessions' names, and
    /// which the Claude desktop app has.
    fn poll_names(&self) {
        let events = self.events();
        let renamed = self.names.lock().expect("names lock").poll(&events);
        let mut apps = self.desktop.lock().expect("desktop lock");
        let desktop = apps.poll(&events);
        // Sessions the Claude app says are blocked on you, marked so in
        // their logs (the tail then reads it, as any event).
        apps.flag_blocked(&self.events_dir);
        drop(apps);
        if renamed || desktop {
            self.bump();
        }
    }

    fn bump(&self) {
        *self.version.lock().expect("version lock") += 1;
        self.changed.notify_all();
    }

    fn titles(&self) -> BTreeMap<String, String> {
        self.names.lock().expect("names lock").titles.clone()
    }

    fn local(&self) -> api::Local {
        api::Local {
            titles: self.titles(),
            desktop: self.desktop.lock().expect("desktop lock").sessions.clone(),
            cursor: self.names.lock().expect("names lock").cursor_chats.clone(),
        }
    }

    /// The events as they are now: a snapshot, worked on without the lock.
    fn events(&self) -> Arc<Vec<Timed>> {
        self.tail.lock().expect("tail lock").events.clone()
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
    // The page and its script and styles are built in and hold nothing
    // private; everything else needs this run's key.
    let public = req.method == "GET"
        && matches!(
            req.path.as_str(),
            "/" | "/index.html" | "/app.css" | "/app.js" | "/d3.min.js" | "/icon.svg"
        );
    if !public && !has_key(req, &shared.key) {
        return respond(stream, 403, "text/plain", &[], NEEDS_KEY);
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
        "/d3.min.js" => respond(
            stream,
            200,
            "text/javascript; charset=utf-8",
            &[],
            D3_JS.as_bytes(),
        ),
        "/icon.svg" => respond(
            stream,
            200,
            "image/svg+xml",
            &[("Cache-Control", "max-age=86400")],
            ICON_SVG.as_bytes(),
        ),
        "/api/graph" => json(
            stream,
            graph_json(shared, req.param("until"), req.param("root")),
        ),
        "/api/image.png" | "/api/image.svg" => {
            let Some(root) = req.param("root") else {
                return respond(stream, 404, "text/plain", &[], b"missing root");
            };
            let svg = req.path.ends_with(".svg");
            let theme = match req.param("theme") {
                Some("dark") => crate::image::Theme::Dark,
                _ => crate::image::Theme::Light,
            };
            let events = shared.events();
            let titles = shared.titles();
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
                &titles,
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

/// `GET /api/graph`'s reply: without `until`, with the timeline of the tree
/// it holds too, so a refresh reduces the events once.
fn graph_json(
    shared: &Shared,
    until: Option<&str>,
    root: Option<&str>,
) -> Result<String, ApiError> {
    api::graph_with(
        &shared.events(),
        until,
        root,
        crate::clock::now(),
        shared.stale_after,
        Environment::Local,
        &shared.local(),
    )
}

/// `POST /api/open?node=<id>`: reopens a session: in the Claude desktop app
/// if the app has it, else in its agent, in a new terminal window. Only the
/// node id comes from the page; the command is worked out from the log, and
/// the app's id for the session from its records. Replies `{"ok": true,
/// "command": …}`, or `{"error": …, "command": …}` with the command to run
/// yourself (in a terminal).
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
    let events = shared.events();
    let graph = api::graph_at(&events, None, crate::clock::now(), shared.stale_after)
        .ok()
        .map(|(graph, _, _)| graph);
    let node = graph.as_ref().and_then(|g| g.nodes.get(id));
    let found = node.and_then(resume::resume);
    let transcript = transcript_of(&events, id);
    let command = found.as_ref().map(open::command_line);
    // The app shows a session it has without its folder, which an older
    // log may not have.
    let desktop = node
        .is_some()
        .then(|| {
            shared
                .desktop
                .lock()
                .expect("desktop lock")
                .sessions
                .get(id)
                .cloned()
        })
        .flatten();
    let r = match (found, desktop) {
        (Some(r), desktop) => Resume { desktop, ..r },
        (None, Some(link)) => match resume::in_desktop_app(link) {
            Some(r) => r,
            None => {
                let error = "That can't be opened: its app's link isn't one Agent Graph opens.";
                return reply(stream, 404, serde_json::json!({ "error": error }));
            }
        },
        (None, None) => {
            let error = "That can't be opened: only sessions whose agent can resume them can.";
            return reply(stream, 404, serde_json::json!({ "error": error }));
        }
    };
    // The app keeps its own sessions; a terminal needs the folder and the
    // conversation to be here.
    let problem = if r.desktop.is_some() {
        None
    } else if !Path::new(&r.cwd).is_dir() {
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
            serde_json::json!({
                "error": if let Some(app) = r.desktop.as_deref().and_then(resume::app_of_link) {
                    format!("Couldn't open it in {app}: {e}.")
                } else {
                    format!("Couldn't open a terminal: {e}.")
                },
                "command": command,
            }),
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

const NEEDS_KEY: &[u8] = b"Open the viewer with the link `agent-graph view` printed: \
it carries this run's key.";

/// A key no one else can guess: 160 random bits from a generator seeded by
/// the OS (the random part of two ULIDs).
fn new_key() -> String {
    let random = || ulid::Ulid::from_datetime(SystemTime::now()).random();
    format!("{:020x}{:020x}", random(), random())
}

/// Whether the request carries the key: in the `X-Agent-Graph-Key` header,
/// or for the event stream (which can't send headers), as `?key=`.
fn has_key(req: &Request, key: &str) -> bool {
    same_key(req.header("x-agent-graph-key"), key)
        || (req.method == "GET" && req.path == "/api/stream" && same_key(req.param("key"), key))
}

/// A connection's place under `MAX_CONNECTIONS`, given back when dropped.
struct Slot<'a>(&'a AtomicUsize);

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Whether `given` is the key, compared in constant time.
fn same_key(given: Option<&str>, key: &str) -> bool {
    let Some(given) = given else { return false };
    given.len() == key.len()
        && given
            .bytes()
            .zip(key.bytes())
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            == 0
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

/// Opens `url` in the default browser (and says nothing if it can't).
pub fn open_browser(url: &str) {
    use std::process::{Command, Stdio};
    let mut cmd = if cfg!(target_os = "macos") {
        Command::new("open")
    } else if cfg!(windows) {
        // Not `cmd /C start`: cmd reads `&` in an address as another command.
        let mut c = Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler");
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
    titles: &BTreeMap<String, String>,
) -> Result<Vec<u8>, ApiError> {
    let (mut graph, _, now) = api::graph_at(events, until, crate::clock::now(), stale_after)?;
    if until.is_none() {
        api::retitle(&mut graph, titles);
    }
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

    /// R7: a handler that panics still gives its connection slot back, or
    /// a few panics would lock everyone out.
    #[test]
    fn a_panicking_handler_gives_its_slot_back() {
        let connections = AtomicUsize::new(1);
        let result = std::panic::catch_unwind(|| {
            let _slot = Slot(&connections);
            panic!("handler failed");
        });
        assert!(result.is_err());
        assert_eq!(connections.load(Ordering::SeqCst), 0);
    }

    /// R23: the events are only locked to take a snapshot: graphs and
    /// timelines are worked out without holding them, so a long history
    /// doesn't hold up new events (or other requests).
    #[test]
    fn requests_are_worked_out_without_holding_the_events() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = String::new();
        for s in 0..3000 {
            for (n, kind) in ["session.started", "session.ended"].iter().enumerate() {
                let i = 2 * s + n;
                log += &format!(
                    r#"{{"v":1,"id":"01K{i:023}","ts":"2026-09-25T10:{:02}:{:02}.{:03}Z","type":"{kind}","node":"x:s{s}","data":{{}}}}"#,
                    i / 60_000 % 60,
                    i / 1000 % 60,
                    i % 1000
                );
                log.push('\n');
            }
        }
        std::fs::write(dir.path().join("x.jsonl"), log).unwrap();
        let shared = Arc::new(Shared {
            tail: Mutex::new(Tail::new(dir.path())),
            names: Mutex::new(Names::default()),
            desktop: Mutex::new(Desktop::new(None, false)),
            version: Mutex::new(0),
            changed: Condvar::new(),
            stale_after: Duration::from_secs(600),
            port: 7777,
            events_dir: dir.path().to_path_buf(),
            launch: |_| Ok(()),
            key: "k".into(),
            connections: AtomicUsize::new(0),
        });
        shared.poll().unwrap();
        assert_eq!(shared.events().len(), 6000);

        type Work = fn(&Shared) -> Result<String, ApiError>;
        let works: [(&str, Work); 2] = [
            ("graph and timeline", |s| graph_json(s, None, Some("x:s0"))),
            ("a step's graph", |s| {
                graph_json(s, Some("01K00000000000000000000100"), Some("x:s0"))
            }),
        ];
        for (name, work) in works {
            let busy = {
                let shared = shared.clone();
                thread::spawn(move || work(&shared).unwrap())
            };
            let (mut free, mut held) = (0u32, 0u32);
            while !busy.is_finished() {
                match shared.tail.try_lock() {
                    Ok(_) => free += 1,
                    Err(_) => held += 1,
                }
                thread::yield_now();
            }
            busy.join().unwrap();
            assert!(
                held * 10 <= free + held,
                "{name}: the events were locked for {held} of {} looks",
                free + held
            );
        }
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
