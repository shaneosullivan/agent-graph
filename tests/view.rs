//! The viewer's HTTP server, over real sockets.

mod common;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::time::{Duration, Instant};

use agent_graph::adapter::Capture;
use common::{SESSION, fixture, translate};

/// Writes the first `n` fixture events into `dir` and starts a viewer on it.
fn start(dir: &Path, n: usize) -> u16 {
    write_events(dir, 0, n);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let key = agent_graph::view::start(dir, listener, Duration::from_secs(1800)).unwrap();
    remember_key(port, key);
    port
}

/// Each test viewer's key, by port, so requests can carry it.
static KEYS: std::sync::Mutex<Vec<(u16, String)>> = std::sync::Mutex::new(Vec::new());

fn remember_key(port: u16, key: String) {
    KEYS.lock().unwrap().push((port, key));
}

fn key_for(port: u16) -> String {
    let keys = KEYS.lock().unwrap();
    keys.iter()
        .find(|(p, _)| *p == port)
        .expect("started")
        .1
        .clone()
}

/// The key header the page sends to the viewer on `port`.
fn key_header(port: u16) -> String {
    format!("X-Agent-Graph-Key: {}\r\n", key_for(port))
}

fn write_events(dir: &Path, from: usize, to: usize) {
    std::fs::create_dir_all(dir).unwrap();
    let events = translate(&fixture("claude-code/session.jsonl"), Capture::default());
    let text: String = events[from..to]
        .iter()
        .map(|e| agent_graph::emit::to_line(e) + "\n")
        .collect();
    agent_graph::store::append(&dir.join("claude-code-s.jsonl"), text.as_bytes()).unwrap();
}

/// A GET from the viewer's own page (with its key).
fn get(port: u16, path: &str, host: &str) -> (u16, String, String) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let key = key_header(port);
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {host}\r\n{key}Connection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let (head, body) = response.split_once("\r\n\r\n").unwrap();
    let status = head.split(' ').nth(1).unwrap().parse().unwrap();
    (status, head.to_string(), body.to_string())
}

#[test]
fn serves_the_page_with_a_strict_policy() {
    let dir = tempfile::tempdir().unwrap();
    let port = start(dir.path(), 5);
    let host = format!("localhost:{port}");

    let (status, head, body) = get(port, "/", &host);
    assert_eq!(status, 200);
    assert!(body.contains("<title>Agent Graph</title>"));
    assert!(head.contains("Content-Security-Policy: default-src 'none'"));
    assert_eq!(get(port, "/app.js", &host).0, 200);
    assert_eq!(get(port, "/app.css", &host).0, 200);
    let (status, head, body) = get(port, "/icon.svg", &host);
    assert_eq!(status, 200);
    assert!(head.contains("Content-Type: image/svg+xml"), "{head}");
    assert!(body.starts_with("<svg"));
    assert_eq!(get(port, "/nope", &host).0, 404);
}

#[test]
fn refuses_other_host_names() {
    let dir = tempfile::tempdir().unwrap();
    let port = start(dir.path(), 5);
    assert_eq!(get(port, "/api/graph", "attacker.example").0, 403);
    assert_eq!(get(port, "/api/graph", &format!("127.0.0.1:{port}")).0, 200);
}

#[test]
fn the_graph_endpoint_carries_the_timeline() {
    let dir = tempfile::tempdir().unwrap();
    let port = start(dir.path(), 9);
    let host = format!("localhost:{port}");

    let (status, _, body) = get(port, "/api/graph", &host);
    assert_eq!(status, 200);
    let graph: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(graph["events"], 9);
    assert_eq!(graph["roots"][0], SESSION);

    // R56: the graph now comes with its tree's timeline.
    let root = SESSION.replace(':', "%3A");
    let (status, _, body) = get(port, &format!("/api/graph?root={root}"), &host);
    assert_eq!(status, 200);
    let graph: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(graph["root"], SESSION);
    let stops = graph["stops"].as_array().unwrap();
    assert_eq!(stops.len(), 9);

    // Step back to the moment the Explore agent started.
    let id = stops[6]["id"].as_str().unwrap();
    let (status, _, body) = get(port, &format!("/api/graph?until={id}&root={root}"), &host);
    assert_eq!(status, 200);
    let past: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        past["nodes"][SESSION]["blocked"]["on"][0],
        format!("{SESSION}/a1f00d")
    );
    assert!(past.get("stops").is_none(), "a step's is the same timeline");

    assert_eq!(get(port, "/api/graph?until=nope", &host).0, 404);
}

#[test]
fn stream_announces_new_events() {
    let dir = tempfile::tempdir().unwrap();
    let port = start(dir.path(), 3);
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    write!(
        stream,
        "GET /api/stream?key={} HTTP/1.1\r\nHost: localhost:{port}\r\n\r\n",
        key_for(port)
    )
    .unwrap();
    let mut reader = BufReader::new(stream);

    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert!(line.starts_with("HTTP/1.1 200"));
    // Skip the headers and the retry hint.
    loop {
        line.clear();
        reader.read_line(&mut line).unwrap();
        if line.starts_with("retry:") {
            break;
        }
    }

    write_events(dir.path(), 3, 5);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        line.clear();
        reader.read_line(&mut line).unwrap();
        if line.trim() == "event: changed" {
            break;
        }
        assert!(Instant::now() < deadline, "no change announced");
    }

    let (_, _, body) = get(port, "/api/graph", &format!("localhost:{port}"));
    let graph: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(graph["events"], 5);
}

// ---------- opening sessions ----------

/// What the viewer asked to open, instead of opening terminal windows.
static OPENED: std::sync::Mutex<Vec<agent_graph::resume::Resume>> =
    std::sync::Mutex::new(Vec::new());

fn record(r: &agent_graph::resume::Resume) -> Result<(), String> {
    OPENED.lock().unwrap().push(r.clone());
    Ok(())
}

fn opened(session_id: &str) -> Vec<agent_graph::resume::Resume> {
    OPENED
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.args.iter().any(|a| a == session_id))
        .cloned()
        .collect()
}

/// A Claude Code session that ran in `cwd`, and ended if `ended`.
fn session_lines(session_id: &str, cwd: &Path, transcript: Option<&Path>, ended: bool) -> String {
    let node = format!("claude-code:{session_id}");
    let source = serde_json::json!({ "provider": "claude-code" });
    let mut data = serde_json::json!({ "cwd": cwd });
    if let Some(t) = transcript {
        data["transcript_path"] = serde_json::json!(t);
    }
    let mut lines = vec![serde_json::json!({
        "v": 1, "id": "01K0000000000000000000000A", "ts": "2026-09-25T10:00:00.000Z",
        "type": "session.started", "node": node, "source": source, "data": data,
    })];
    if ended {
        lines.push(serde_json::json!({
            "v": 1, "id": "01K0000000000000000000000B", "ts": "2026-09-25T10:05:00.000Z",
            "type": "session.ended", "node": node, "source": source, "data": {},
        }));
    }
    lines.iter().map(|l| format!("{l}\n")).collect()
}

fn start_recording(dir: &Path, lines: &str) -> u16 {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("claude-code-s.jsonl"), lines).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let key =
        agent_graph::view::start_with(dir, listener, Duration::from_secs(1800), record).unwrap();
    remember_key(port, key);
    port
}

fn post(port: u16, path: &str, origin: Option<&str>) -> (u16, serde_json::Value) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let origin = origin.map_or(String::new(), |o| format!("Origin: {o}\r\n"));
    let key = key_header(port);
    write!(
        stream,
        "POST {path} HTTP/1.1\r\nHost: localhost:{port}\r\n{origin}{key}Content-Length: 0\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let (head, body) = response.split_once("\r\n\r\n").unwrap();
    let status = head.split(' ').nth(1).unwrap().parse().unwrap();
    (status, serde_json::from_str(body).unwrap_or_default())
}

#[test]
fn opens_a_session_only_for_the_viewers_own_page() {
    let id = "0d0e0f10-aaaa-4bbb-8ccc-000000000001";
    let dir = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let transcript = project.path().join("transcript.jsonl");
    std::fs::write(&transcript, "").unwrap();
    let port = start_recording(
        &dir.path().join("events"),
        &session_lines(id, project.path(), Some(&transcript), true),
    );
    let path = format!("/api/open?node=claude-code%3A{id}");
    let own = format!("http://localhost:{port}");

    // The graph offers it...
    let (_, _, body) = get(
        port,
        &format!("/api/graph?root=claude-code%3A{id}"),
        &format!("localhost:{port}"),
    );
    let graph: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        graph["open"][format!("claude-code:{id}")],
        serde_json::json!({ "app": "Claude Code", "copy": false })
    );

    // ...but only this viewer's page may take it up.
    assert_eq!(post(port, &path, None).0, 403, "no Origin");
    assert_eq!(post(port, &path, Some("https://evil.example")).0, 403);
    assert_eq!(post(port, &path, Some("null")).0, 403);
    assert_eq!(get(port, &path, &format!("localhost:{port}")).0, 405);
    assert!(opened(id).is_empty(), "nothing opened yet");

    let (status, reply) = post(port, &path, Some(&own));
    assert_eq!(status, 200, "{reply}");
    assert_eq!(reply["ok"], true);
    // Named by its full path when it's on PATH (R8), so just its gist.
    let command = reply["command"].as_str().unwrap();
    assert!(command.ends_with(&format!(" --resume {id}")), "{command}");
    assert!(command.contains("claude"), "{command}");
    let runs = opened(id);
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].program, "claude");
    assert_eq!(
        runs[0].args,
        ["--resume", id],
        "it ended: resume it, not a copy"
    );
    assert_eq!(runs[0].cwd, project.path().to_str().unwrap());

    let (status, reply) = post(port, "/api/open?node=claude-code%3Anope", Some(&own));
    assert_eq!(status, 404);
    assert!(reply["error"].is_string());
}

#[test]
fn a_running_session_opens_as_a_copy() {
    let id = "0d0e0f10-aaaa-4bbb-8ccc-000000000002";
    let dir = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let port = start_recording(
        &dir.path().join("events"),
        &session_lines(id, project.path(), None, false),
    );
    let (status, _) = post(
        port,
        &format!("/api/open?node=claude-code%3A{id}"),
        Some(&format!("http://localhost:{port}")),
    );
    assert_eq!(status, 200);
    assert_eq!(opened(id)[0].args, ["--resume", id, "--fork-session"]);
}

#[test]
fn a_session_that_is_not_on_this_computer_is_not_opened() {
    let dir = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    for (id, lines) in [
        (
            "0d0e0f10-aaaa-4bbb-8ccc-000000000003",
            session_lines(
                "0d0e0f10-aaaa-4bbb-8ccc-000000000003",
                &project.path().join("gone"),
                None,
                true,
            ),
        ),
        (
            "0d0e0f10-aaaa-4bbb-8ccc-000000000004",
            session_lines(
                "0d0e0f10-aaaa-4bbb-8ccc-000000000004",
                project.path(),
                Some(&project.path().join("deleted.jsonl")),
                true,
            ),
        ),
    ] {
        let port = start_recording(&dir.path().join(id), &lines);
        let (status, reply) = post(
            port,
            &format!("/api/open?node=claude-code%3A{id}"),
            Some(&format!("http://localhost:{port}")),
        );
        assert_eq!(status, 404, "{reply}");
        assert!(reply["error"].is_string());
        assert!(
            reply["command"].as_str().unwrap().contains(id),
            "says what to run instead"
        );
        assert!(opened(id).is_empty());
    }
}

/// R7: loopback isn't "this user". The page is public, but no data, and no
/// Resume, without this run's key, which only the page's own origin sends.
#[test]
fn nothing_private_is_served_without_the_viewers_key() {
    let dir = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let id = "0d0e0f10-aaaa-4bbb-8ccc-0000000000f7";
    let port = start_recording(
        &dir.path().join("events"),
        &session_lines(id, project.path(), None, true),
    );
    let key = key_for(port);
    let host = format!("localhost:{port}");
    let wrong = "0".repeat(key.len());

    for path in ["/", "/app.js", "/app.css", "/icon.svg"] {
        assert_eq!(get_with(port, path, &host, None).0, 200, "{path}");
    }
    for header in [
        None,
        Some(format!("X-Agent-Graph-Key: {wrong}")),
        // A cookie isn't the key: browsers send cookies to every localhost port.
        Some(format!("Cookie: agent_graph_{port}={key}")),
    ] {
        for path in [
            "/api/graph",
            "/api/info",
            "/api/stream",
            "/api/image.svg?root=x",
        ] {
            let (status, _, body) = get_with(port, path, &host, header.as_deref());
            assert_eq!(status, 403, "{path} served with {header:?}: {body}");
        }
    }
    let own = Some(format!("X-Agent-Graph-Key: {key}"));
    assert_eq!(get_with(port, "/api/graph", &host, own.as_deref()).0, 200);
    // Only the event stream, which can't send headers, takes it in the URL.
    assert_eq!(
        get_with(port, &format!("/api/graph?key={key}"), &host, None).0,
        403
    );

    // Resume: the right Origin isn't enough (any local program can send it).
    let path = format!("/api/open?node=claude-code%3A{id}");
    let origin = format!("Origin: http://{host}");
    for extra in [
        String::new(),
        format!("X-Agent-Graph-Key: {wrong}\r\n"),
        format!("Cookie: agent_graph_{port}={key}\r\n"),
    ] {
        let status = raw_post(port, &path, &format!("{origin}\r\n{extra}"));
        assert_eq!(status, 403, "opened with {extra:?}");
    }
    assert_eq!(
        raw_post(port, &format!("{path}&key={key}"), &format!("{origin}\r\n")),
        403
    );
    assert!(opened(id).is_empty(), "nothing was launched");
    assert_eq!(post(port, &path, Some(&format!("http://{host}"))).0, 200);
    assert_eq!(opened(id).len(), 1);
}

/// A POST with exactly `headers` (each ending in CRLF); returns the status.
fn raw_post(port: u16, path: &str, headers: &str) -> u16 {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        stream,
        "POST {path} HTTP/1.1\r\nHost: localhost:{port}\r\n{headers}Content-Length: 0\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response.split(' ').nth(1).unwrap().parse().unwrap()
}

/// A GET with an optional extra header line.
fn get_with(port: u16, path: &str, host: &str, header: Option<&str>) -> (u16, String, String) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let header = header.map_or(String::new(), |h| format!("{h}\r\n"));
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {host}\r\n{header}Connection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = Vec::new();
    let _ = stream.read_to_end(&mut response);
    let response = String::from_utf8_lossy(&response).into_owned();
    let (head, body) = response.split_once("\r\n\r\n").unwrap_or((&response, ""));
    let status = head
        .split(' ')
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    (status, head.to_string(), body.to_string())
}

/// R7: connections past the limit are closed rather than each taking a
/// thread, and those already open still get answers.
#[test]
fn connections_past_the_limit_are_closed() {
    let dir = tempfile::tempdir().unwrap();
    let port = start(dir.path(), 3);
    raise_open_file_limit();
    let idle: Vec<TcpStream> = (0..agent_graph::view::MAX_CONNECTIONS)
        .map(|_| TcpStream::connect(("127.0.0.1", port)).unwrap())
        .collect();
    std::thread::sleep(Duration::from_millis(200));
    let mut extra = TcpStream::connect(("127.0.0.1", port)).unwrap();
    extra
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut buf = [0u8; 1];
    // Closed by the server (not just waiting for a request, which would
    // time out here instead).
    let read = extra.read(&mut buf);
    let closed = match &read {
        Ok(0) => true,
        Err(e) => e.kind() == std::io::ErrorKind::ConnectionReset,
        Ok(_) => false,
    };
    assert!(closed, "the connection past the limit was kept: {read:?}");
    drop(idle);
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(get(port, "/api/graph", &format!("localhost:{port}")).0, 200);
}

/// Each connection is two open files in this process (both ends), which is
/// more than macOS allows by default.
fn raise_open_file_limit() {
    #[cfg(unix)]
    unsafe {
        let mut limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        if libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) == 0 {
            limit.rlim_cur = limit.rlim_max.min(4096);
            libc::setrlimit(libc::RLIMIT_NOFILE, &limit);
        }
    }
}

/// R7: browsers try `localhost` as [::1] first. If another program holds
/// [::1] at the viewer's port, the viewer won't start (its link or a typed
/// `localhost` could go there), and the link it prints names 127.0.0.1.
#[test]
fn the_viewer_wont_share_its_port_on_ipv6_localhost() {
    assert!(agent_graph::view::link(7777, "k3y").starts_with("http://127.0.0.1:7777/?key=k3y"));
    let Ok(squatter) = TcpListener::bind(("::1", 0)) else {
        return; // no IPv6 here, so nothing can listen there either
    };
    let port = squatter.local_addr().unwrap().port();
    let err = agent_graph::view::bind(port).expect_err("started beside a squatter");
    assert!(err.contains("[::1]"), "{err}");
    drop(squatter);
    let listeners = agent_graph::view::bind(port).unwrap();
    assert_eq!(listeners.len(), 2, "listens on both");
}
