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
