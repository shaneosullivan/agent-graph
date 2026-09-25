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
    agent_graph::view::start(dir, listener, Duration::from_secs(1800)).unwrap();
    port
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

fn get(port: u16, path: &str, host: &str) -> (u16, String, String) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
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
fn graph_and_timeline_endpoints() {
    let dir = tempfile::tempdir().unwrap();
    let port = start(dir.path(), 9);
    let host = format!("localhost:{port}");

    let (status, _, body) = get(port, "/api/graph", &host);
    assert_eq!(status, 200);
    let graph: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(graph["events"], 9);
    assert_eq!(graph["roots"][0], SESSION);

    let root = SESSION.replace(':', "%3A");
    let (status, _, body) = get(port, &format!("/api/timeline?root={root}"), &host);
    assert_eq!(status, 200);
    let timeline: serde_json::Value = serde_json::from_str(&body).unwrap();
    let stops = timeline["stops"].as_array().unwrap();
    assert_eq!(stops.len(), 9);

    // Step back to the moment the Explore agent started.
    let id = stops[6]["id"].as_str().unwrap();
    let (status, _, body) = get(port, &format!("/api/graph?until={id}"), &host);
    assert_eq!(status, 200);
    let past: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        past["nodes"][SESSION]["blocked"]["on"][0],
        format!("{SESSION}/a1f00d")
    );

    assert_eq!(get(port, "/api/graph?until=nope", &host).0, 404);
    assert_eq!(get(port, "/api/timeline?root=x%3Anope", &host).0, 404);
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
        "GET /api/stream HTTP/1.1\r\nHost: localhost:{port}\r\n\r\n"
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
    agent_graph::view::start_with(dir, listener, Duration::from_secs(1800), record).unwrap();
    port
}

fn post(port: u16, path: &str, origin: Option<&str>) -> (u16, serde_json::Value) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let origin = origin.map_or(String::new(), |o| format!("Origin: {o}\r\n"));
    write!(
        stream,
        "POST {path} HTTP/1.1\r\nHost: localhost:{port}\r\n{origin}Content-Length: 0\r\nConnection: close\r\n\r\n"
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
    let (_, _, body) = get(port, "/api/graph", &format!("localhost:{port}"));
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
    assert!(
        reply["command"]
            .as_str()
            .unwrap()
            .ends_with(&format!("claude --resume {id}"))
    );
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
