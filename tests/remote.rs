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

/// A session's start, in `cwd`.
fn started(session: &str, cwd: &str) -> String {
    format!(
        r#"{{"v":1,"id":"01K00000000000000000000S{session}","ts":"2026-09-25T10:00:00.000Z","type":"session.started","node":"claude-code:{session}","data":{{"cwd":"{cwd}"}}}}"#
    ) + "\n"
}

/// R10: sharing "current" means the session this runs in. When that isn't
/// recorded, it fails, rather than sharing whichever session is newest
/// (another project's, perhaps) with anyone who has the link.
#[test]
fn sharing_the_current_session_never_falls_back_to_another() {
    let (port, requests) = mock_site();
    let home = tempfile::tempdir().unwrap();
    let events = home.path().join("events");
    std::fs::create_dir_all(&events).unwrap();
    std::fs::write(
        events.join("claude-code-secret.jsonl"),
        started("secret", "/work/confidential"),
    )
    .unwrap();
    let here = tempfile::tempdir().unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_agent-graph"))
        .args(["watch-remote", "--session", "current", "--url"])
        .arg(format!("http://127.0.0.1:{port}"))
        .env("AGENT_GRAPH_HOME", home.path())
        .env("CLAUDE_CODE_SESSION_ID", "not-recorded")
        .current_dir(here.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Sharing keeps going until stopped; refusing exits straight away.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while child.try_wait().unwrap().is_none() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    let out = child.wait_with_output().unwrap();
    assert!(!out.status.success(), "it shared something");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("--session"), "says how to choose: {err}");
    assert!(
        requests.recv_timeout(Duration::from_millis(500)).is_err(),
        "nothing was sent to the site"
    );

    // Recorded: that session is shared, and named.
    let mut child = Command::new(env!("CARGO_BIN_EXE_agent-graph"))
        .args(["watch-remote", "--session", "current", "--url"])
        .arg(format!("http://127.0.0.1:{port}"))
        .env("AGENT_GRAPH_HOME", home.path())
        .env("CLAUDE_CODE_SESSION_ID", "secret")
        .current_dir(here.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let created = requests
        .recv_timeout(Duration::from_secs(5))
        .expect("shared");
    assert!(created.body.contains("claude-code:secret"));
    std::thread::sleep(Duration::from_millis(300));
    let _ = child.kill();
    let out = child.wait_with_output().unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("session claude-code:secret (in /work/confidential)"),
        "{err}"
    );
}

/// Runs `watch-remote` with `args` and `env` for up to two seconds (sharing
/// keeps going until stopped), then stops it. Returns its stderr.
fn watch(home: &std::path::Path, port: u16, args: &[&str], env: &[(&str, &str)]) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_agent-graph"))
        .arg("watch-remote")
        .args(args)
        .arg("--url")
        .arg(format!("http://127.0.0.1:{port}"))
        .env("AGENT_GRAPH_HOME", home)
        .envs(env.iter().copied())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while child.try_wait().unwrap().is_none() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    String::from_utf8_lossy(&child.wait_with_output().unwrap().stderr).into_owned()
}

/// R10: a short --session prefix is matched against sessions only: one that
/// happens to match an agent in another project's session shares nothing.
#[test]
fn a_session_prefix_never_matches_another_sessions_agent() {
    let (port, requests) = mock_site();
    let home = tempfile::tempdir().unwrap();
    let events = home.path().join("events");
    std::fs::create_dir_all(&events).unwrap();
    let agent = r#"{"v":1,"id":"01K0000000000000000000AG01","ts":"2026-09-25T10:00:01.000Z","type":"agent.spawned","node":"claude-code:9d1e/a3f91c2e5b","parent":"claude-code:9d1e","data":{}}"#;
    std::fs::write(
        events.join("claude-code-9d1e.jsonl"),
        started("9d1e", "/work/other") + agent + "\n",
    )
    .unwrap();
    let err = watch(home.path(), port, &["--session", "a3f9"], &[]);
    assert!(err.contains("no session matches"), "{err}");
    assert!(
        requests.recv_timeout(Duration::from_millis(300)).is_err(),
        "shared"
    );
}

/// R10: what's printed about the session comes from the log, so it's
/// cleaned of control characters, like everything else printed from it.
#[test]
fn the_shared_sessions_description_is_cleaned() {
    let (port, requests) = mock_site();
    let home = tempfile::tempdir().unwrap();
    let events = home.path().join("events");
    std::fs::create_dir_all(&events).unwrap();
    let cwd = r"/tmp/x\u001b]0;PWNED\u0007\u001b[2J\u202egnp.exe";
    std::fs::write(events.join("claude-code-evil.jsonl"), started("evil", cwd)).unwrap();
    let err = watch(
        home.path(),
        port,
        &["--session", "current"],
        &[("CLAUDE_CODE_SESSION_ID", "evil")],
    );
    requests
        .recv_timeout(Duration::from_secs(2))
        .expect("shared");
    assert!(err.contains("session claude-code:evil"), "{err}");
    assert!(
        !err.contains(['\u{1b}', '\u{7}', '\u{202e}']),
        "control characters reached the terminal: {err:?}"
    );
}

/// A session's start, with its parent.
fn started_under(session: &str, parent: &str) -> String {
    format!(
        r#"{{"v":1,"id":"01K00000000000000000000C{session}","ts":"2026-09-25T10:00:01.000Z","type":"session.started","node":"{session}","parent":"{parent}","data":{{"cwd":"/work/app"}}}}"#
    ) + "\n"
}

/// R13: sharing one session shares the sessions (and `run`s) under it too,
/// each from its own file, including ones that start later; and nothing
/// from sessions outside it.
#[test]
fn sharing_a_session_includes_the_sessions_under_it() {
    let (port, requests) = mock_site();
    let home = tempfile::tempdir().unwrap();
    let events = home.path().join("events");
    std::fs::create_dir_all(&events).unwrap();
    let write = |file: &str, text: &str| {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(events.join(file))
            .unwrap();
        f.write_all(text.as_bytes()).unwrap();
    };
    write("claude-code-parent.jsonl", &started("parent", "/work/app"));
    write(
        "claude-code-child.jsonl",
        &started_under("claude-code:child", "claude-code:parent"),
    );
    write(
        "run-r1.jsonl",
        &started_under("run:r1", "claude-code:parent"),
    );
    write(
        "claude-code-other.jsonl",
        &started("other", "/work/elsewhere"),
    );

    let _running = Stopped(
        Command::new(env!("CARGO_BIN_EXE_agent-graph"))
            .args(["watch-remote", "--session", "parent", "--url"])
            .arg(format!("http://127.0.0.1:{port}"))
            .env("AGENT_GRAPH_HOME", home.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let created = requests
        .recv_timeout(Duration::from_secs(5))
        .expect("shared");
    for node in ["claude-code:parent", "claude-code:child", "run:r1"] {
        assert!(
            created.body.contains(&format!("\"node\":\"{node}\"")),
            "{node} missing"
        );
    }
    assert!(
        !created.body.contains("claude-code:other"),
        "shared another session"
    );

    // A session that starts under it later is shared too; the other isn't.
    write(
        "claude-code-other.jsonl",
        &line(7).replace("x:s", "claude-code:other"),
    );
    write(
        "claude-code-late.jsonl",
        &started_under("claude-code:late", "claude-code:parent"),
    );
    let mut sent = String::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !sent.contains("claude-code:late") && std::time::Instant::now() < deadline {
        if let Ok(r) = requests.recv_timeout(Duration::from_millis(200)) {
            sent.push_str(&r.body);
        }
    }
    assert!(
        sent.contains("claude-code:late"),
        "the late child wasn't shared: {sent}"
    );
    assert!(
        !sent.contains("claude-code:other"),
        "shared another session"
    );
}

/// A child process that's stopped when this is dropped, however the test ends.
struct Stopped(std::process::Child);

impl Drop for Stopped {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// R13: a session that moves out of the shared tree (resumed from another
/// session's shell) stops being shared, even when nothing else changes.
#[test]
fn a_session_that_leaves_the_shared_tree_stops_being_shared() {
    let (port, requests) = mock_site();
    let home = tempfile::tempdir().unwrap();
    let events = home.path().join("events");
    std::fs::create_dir_all(&events).unwrap();
    let write = |file: &str, text: &str| {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(events.join(file))
            .unwrap();
        f.write_all(text.as_bytes()).unwrap();
    };
    write("claude-code-parent.jsonl", &started("parent", "/work/app"));
    write(
        "claude-code-child.jsonl",
        &started_under("claude-code:child", "claude-code:parent"),
    );

    let _running = Stopped(
        Command::new(env!("CARGO_BIN_EXE_agent-graph"))
            .args(["watch-remote", "--session", "parent", "--url"])
            .arg(format!("http://127.0.0.1:{port}"))
            .env("AGENT_GRAPH_HOME", home.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let created = requests
        .recv_timeout(Duration::from_secs(5))
        .expect("shared");
    assert!(created.body.contains("claude-code:child"));

    // Resumed under another session: its later events aren't ours to share.
    let moved = started_under("claude-code:child", "claude-code:elsewhere")
        .replace("01K00000000000000000000C", "01K00000000000000000000M")
        .replace("10:00:01", "10:05:00");
    write("claude-code-child.jsonl", &moved);
    write(
        "claude-code-child.jsonl",
        &line(9)
            .replace("x:s", "claude-code:child")
            .replace("working", "input_required"),
    );
    let mut sent = String::new();
    while let Ok(r) = requests.recv_timeout(Duration::from_secs(3)) {
        sent.push_str(&r.body);
    }
    assert!(
        !sent.contains("claude-code:elsewhere"),
        "sent the move: {sent}"
    );
    assert!(
        !sent.contains("input_required"),
        "kept sharing the moved session: {sent}"
    );
}

/// Like `mock_site`, but each append waits for the test to say what status
/// to reply with, so the test can change the log before the reply.
fn scripted_site() -> (u16, mpsc::Receiver<Request>, mpsc::Sender<u16>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = mpsc::channel();
    let (reply_tx, reply_rx) = mpsc::channel::<u16>();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let request = read_request(&stream);
            let reply = if request.path == "/api/logs" {
                let body = r#"{"id":"abc123def456","url":"https://site.example/l/abc123def456","writeToken":"the-key"}"#;
                if tx.send(request).is_err() {
                    return;
                }
                format!(
                    "HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
            } else {
                if tx.send(request).is_err() {
                    return;
                }
                let Ok(status) = reply_rx.recv() else { return };
                format!(
                    "HTTP/1.1 {status} Whatever\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                )
            };
            let _ = (&stream).write_all(reply.as_bytes());
        }
    });
    (port, rx, reply_tx)
}

/// R16: a failed append is retried with exactly the same bytes, even when
/// new lines have arrived meanwhile; they follow in the next chunk. (A
/// bigger retry at the same offset would be missed by viewers that had the
/// first, and lost if the first landed late.)
#[test]
fn a_retried_append_sends_the_same_bytes() {
    let (port, requests, replies) = scripted_site();
    let home = tempfile::tempdir().unwrap();
    let events = home.path().join("events");
    std::fs::create_dir_all(&events).unwrap();
    let file = events.join("x-s.jsonl");
    let first = line(1);
    std::fs::write(&file, &first).unwrap();
    let _running = Stopped(
        Command::new(env!("CARGO_BIN_EXE_agent-graph"))
            .args(["watch-remote", "--password=", "--url"])
            .arg(format!("http://127.0.0.1:{port}"))
            .env("AGENT_GRAPH_HOME", home.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let wait = Duration::from_secs(10);
    assert_eq!(requests.recv_timeout(wait).unwrap().body, first);

    agent_graph::store::append(&file, line(2).as_bytes()).unwrap();
    let tried = requests.recv_timeout(wait).unwrap();
    assert_eq!(tried.body, line(2));
    // More arrives before the site's reply, which is a failure. (Two lines,
    // so the chunk after the retry can't be the retry's length by chance.)
    agent_graph::store::append(&file, (line(3) + &line(4)).as_bytes()).unwrap();
    replies.send(500).unwrap();

    let retried = requests.recv_timeout(wait).unwrap();
    assert_eq!(retried.path, tried.path, "the same offset");
    assert_eq!(retried.body, tried.body, "the same bytes");
    replies.send(204).unwrap();

    let next = requests.recv_timeout(wait).unwrap();
    assert_eq!(
        next.path,
        format!(
            "/api/logs/abc123def456/append?offset={}",
            first.len() + line(2).len()
        )
    );
    assert_eq!(next.body, line(3) + &line(4));
    replies.send(204).unwrap();
}

/// Event `n`, `n` seconds in, for a session that changes every 100.
fn numbered(n: u32) -> String {
    let at = std::time::UNIX_EPOCH + Duration::from_secs(1_790_000_000 + u64::from(n));
    format!(
        r#"{{"v":1,"id":"01K{n:023}","ts":"{}","type":"status","node":"x:s{}","data":{{"state":"working"}}}}"#,
        humantime::format_rfc3339_millis(at),
        n / 100
    ) + "\n"
}

/// Keyframes: a long log is shared from its last keyframe but one, a
/// keyframe starts a chunk, and as the log grows, once another is sent, the
/// site's asked to delete what's before the one before.
#[test]
fn a_long_log_is_shared_from_its_last_keyframe_but_one() {
    let (port, requests) = mock_site();
    let home = tempfile::tempdir().unwrap();
    let events = home.path().join("events");
    std::fs::create_dir_all(&events).unwrap();
    let file = events.join("x-s.jsonl");
    std::fs::write(&file, (0..2500).map(numbered).collect::<String>()).unwrap();
    let _running = Stopped(
        Command::new(env!("CARGO_BIN_EXE_agent-graph"))
            .args(["watch-remote", "--password=", "--url"])
            .arg(format!("http://127.0.0.1:{port}"))
            .env("AGENT_GRAPH_HOME", home.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let wait = Duration::from_secs(20);
    let is_keyframe = |line: &str| line.contains(r#""type":"keyframe""#);
    let event_lines = |body: &str| {
        body.lines()
            .filter(|l| !is_keyframe(l))
            .map(String::from)
            .collect::<Vec<_>>()
    };

    // A keyframe, then events 1000 to 1999; a keyframe, then 2000 to 2499.
    let create = requests.recv_timeout(wait).unwrap();
    assert_eq!(create.path, "/api/logs");
    assert!(is_keyframe(create.body.lines().next().unwrap()));
    assert_eq!(
        event_lines(&create.body).concat(),
        (1000..2000)
            .map(|n| numbered(n).trim_end().to_string())
            .collect::<String>()
    );
    let second = requests.recv_timeout(wait).unwrap();
    assert_eq!(
        second.path,
        format!("/api/logs/abc123def456/append?offset={}", create.body.len())
    );
    assert!(is_keyframe(second.body.lines().next().unwrap()));
    assert_eq!(event_lines(&second.body).len(), 500);

    // Five hundred more: another keyframe, and the site's asked to delete
    // what's before the one before (the second chunk).
    agent_graph::store::append(
        &file,
        (2500..3000).map(numbered).collect::<String>().as_bytes(),
    )
    .unwrap();
    let mut appended = String::new();
    let trim = loop {
        let r = requests.recv_timeout(wait).unwrap();
        if r.path.contains("/trim") {
            break r;
        }
        appended.push_str(&r.body);
    };
    assert_eq!(event_lines(&appended).len(), 500);
    assert!(appended.lines().last().is_some_and(is_keyframe));
    assert_eq!(
        trim.path,
        format!("/api/logs/abc123def456/trim?before={}", create.body.len())
    );
    assert_eq!(trim.headers["authorization"], "Bearer the-key");

    // Two more keyframes' worth: the site's asked to let go of what's
    // before the last keyframe as soon as the next is stored, before the one
    // after is sent (so it never holds three).
    agent_graph::store::append(
        &file,
        (3000..5300).map(numbered).collect::<String>().as_bytes(),
    )
    .unwrap();
    let mut requested: Vec<Request> = Vec::new();
    while requested
        .iter()
        .filter(|r| r.path.contains("/trim"))
        .count()
        < 2
    {
        requested.push(requests.recv_timeout(wait).unwrap());
    }
    let kinds: Vec<&str> = requested
        .iter()
        .map(
            |r| match (r.path.contains("/trim"), r.body.lines().next()) {
                (true, _) => "trim",
                (false, Some(line)) if is_keyframe(line) => "keyframe",
                _ => "events",
            },
        )
        .collect();
    assert_eq!(kinds, ["events", "keyframe", "trim", "keyframe", "trim"]);
}

/// The site deletes a log that has had no new events for a week: a trim
/// refused for that (410) stops the share, saying so.
#[test]
fn an_expired_log_stops_the_share() {
    let (port, requests, replies) = scripted_site();
    let home = tempfile::tempdir().unwrap();
    let events = home.path().join("events");
    std::fs::create_dir_all(&events).unwrap();
    let file = events.join("x-s.jsonl");
    std::fs::write(&file, (0..2500).map(numbered).collect::<String>()).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_agent-graph"))
        .args(["watch-remote", "--password=", "--url"])
        .arg(format!("http://127.0.0.1:{port}"))
        .env("AGENT_GRAPH_HOME", home.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let wait = Duration::from_secs(20);
    requests.recv_timeout(wait).unwrap();
    // The rest of the start, then another keyframe's worth.
    assert!(
        requests
            .recv_timeout(wait)
            .unwrap()
            .path
            .contains("/append")
    );
    replies.send(204).unwrap();
    agent_graph::store::append(
        &file,
        (2500..3000).map(numbered).collect::<String>().as_bytes(),
    )
    .unwrap();
    loop {
        let r = requests.recv_timeout(wait).unwrap();
        if r.path.contains("/trim") {
            replies.send(410).unwrap();
            break;
        }
        replies.send(204).unwrap();
    }
    // (Failing, not hanging, if it doesn't stop.)
    let mut status = None;
    for _ in 0..200 {
        status = child.try_wait().unwrap();
        if status.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let Some(status) = status else {
        let _ = child.kill();
        let _ = child.wait();
        panic!("it didn't stop");
    };
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    assert!(!status.success());
    assert!(stderr.contains("no new events for a week"), "{stderr}");
}

/// R46: the site accepts passwords of up to 1024 bytes (of UTF-8), and
/// unlocks with no longer one; a longer one is refused before anything's
/// sent, or saved.
#[test]
fn a_password_too_long_for_the_site_is_refused() {
    let (port, requests) = mock_site();
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join("events")).unwrap();
    // 1025 bytes, in 513 characters.
    let long = format!("--password={}", "é".repeat(512) + "x");
    let err = watch(home.path(), port, &[&long], &[]);
    assert!(err.contains("at most 1024 bytes"), "{err}");
    let err = watch(home.path(), port, &[&long, "--save-default-password"], &[]);
    assert!(err.contains("at most 1024 bytes"), "{err}");
    assert!(!home.path().join("remote.json").exists(), "not saved");
    assert!(
        requests.recv_timeout(Duration::from_millis(300)).is_err(),
        "nothing sent"
    );

    // 1024 is fine.
    let err = watch(
        home.path(),
        port,
        &[&format!("--password={}", "é".repeat(512))],
        &[],
    );
    let create = requests
        .recv_timeout(Duration::from_secs(2))
        .unwrap_or_else(|_| panic!("not shared: {err}"));
    assert_eq!(create.path, "/api/logs");
}

/// R47: a redirect isn't followed. (One would carry the password header on
/// to wherever it points, over plain HTTP too: only `Authorization` and
/// cookies are dropped.) The site's API never redirects.
#[test]
fn a_redirect_is_not_followed() {
    let (elsewhere, followed) = mock_site();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            read_request(&stream);
            let reply = format!(
                "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{elsewhere}/api/logs\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            let _ = (&stream).write_all(reply.as_bytes());
        }
    });
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join("events")).unwrap();
    let err = watch(home.path(), port, &["--password=s3cret"], &[]);
    assert!(err.contains("302"), "{err}");
    assert!(
        followed.recv_timeout(Duration::from_millis(300)).is_err(),
        "followed the redirect"
    );
}
