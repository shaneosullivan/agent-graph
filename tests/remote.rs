//! `agent-graph watch-remote` against a mock of the site's API.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

#[derive(Debug)]
struct Request {
    path: String,
    headers: HashMap<String, String>,
    body: String,
}

/// Serves the API on a free port: creating a log returns id `abc123def456`
/// and write key `the-key`; appends return 204. Every request is sent on.
fn mock_site() -> (u16, mpsc::Receiver<Request>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let request = read_request(&stream);
            let reply = if request.path == "/api/logs" {
                let body = r#"{"id":"abc123def456","url":"https://site.example/l/abc123def456","writeToken":"the-key"}"#;
                format!(
                    "HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
            } else {
                "HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n".to_string()
            };
            (&stream).write_all(reply.as_bytes()).unwrap();
            if tx.send(request).is_err() {
                return;
            }
        }
    });
    (port, rx)
}

fn read_request(stream: &TcpStream) -> Request {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let path = line.split(' ').nth(1).unwrap_or_default().to_string();
    let mut headers = HashMap::new();
    loop {
        line.clear();
        reader.read_line(&mut line).unwrap();
        let Some((k, v)) = line.trim_end().split_once(':') else {
            break;
        };
        headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
    }
    let len: usize = headers
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0; len];
    reader.read_exact(&mut body).unwrap();
    Request {
        path,
        headers,
        body: String::from_utf8(body).unwrap(),
    }
}

fn line(n: u32) -> String {
    format!(
        r#"{{"v":1,"id":"01K00000000000000000000{n:03}","ts":"2026-09-25T10:00:00.000Z","type":"status","node":"x:s","data":{{"state":"working"}}}}"#
    ) + "\n"
}

#[test]
fn shares_the_log_then_appends_only_new_lines_with_the_key() {
    let (port, requests) = mock_site();
    let home = tempfile::tempdir().unwrap();
    let events = home.path().join("events");
    std::fs::create_dir_all(&events).unwrap();
    let file = events.join("x-s.jsonl");
    let first = line(1) + &line(2);
    std::fs::write(&file, &first).unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_agent-graph"))
        .args([
            "watch-remote",
            &format!("--url=http://127.0.0.1:{port}"),
            "--password=pässword",
        ])
        .env("AGENT_GRAPH_HOME", home.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    // The link comes first, straight away.
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut link = String::new();
    stdout.read_line(&mut link).unwrap();
    assert_eq!(link.trim(), "https://site.example/l/abc123def456");

    let create = requests.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(create.path, "/api/logs");
    assert_eq!(create.body, first, "the existing log");
    assert_eq!(create.headers["x-agent-graph-source"], "watch");
    assert_eq!(
        create.headers["x-agent-graph-password"], "cMOkc3N3b3Jk",
        "base64url of the password"
    );
    assert!(!create.headers.contains_key("authorization"));

    // New lines follow, alone, at the right offset, with the key.
    agent_graph::store::append(&file, line(3).as_bytes()).unwrap();
    let append = requests.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(
        append.path,
        format!("/api/logs/abc123def456/append?offset={}", first.len())
    );
    assert_eq!(append.headers["authorization"], "Bearer the-key");
    assert_eq!(append.body, line(3));

    child.kill().unwrap();
    child.wait().unwrap();
    // The key was only ever in memory.
    let saved: Vec<_> = walk(home.path());
    assert!(saved.iter().all(|text| !text.contains("the-key")));
}

fn walk(dir: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk(&path));
        } else {
            out.push(std::fs::read_to_string(&path).unwrap_or_default());
        }
    }
    out
}
