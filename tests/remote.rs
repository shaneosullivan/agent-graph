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
            if answered_alive(&stream, &request) {
                continue;
            }
            let json = |status: &str, body: &str| {
                format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
            };
            let reply = if request.path == "/api/logs" {
                json(
                    "201 Created",
                    r#"{"id":"abc123def456","url":"https://site.example/watch","writeToken":"the-key","accountStatus":"active"}"#,
                )
            } else if request.path == "/api/cli/token" {
                json(
                    "200 OK",
                    r#"{"token":"agt_from_login","email":"me@example.com"}"#,
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

/// Logged in to the site on `port`, as agent-graph would be after logging in.
fn logged_in(home: &std::path::Path, port: u16) {
    logged_in_as(home, port, "me@example.com");
}

fn logged_in_as(home: &std::path::Path, port: u16, email: &str) {
    let account = serde_json::json!({
        "site": format!("http://127.0.0.1:{port}"),
        "token": "agt_test",
        "email": email,
    });
    std::fs::write(home.join("account.json"), account.to_string()).unwrap();
}

/// Answers `request` if it's watch-remote saying it's still running (every
/// minute or so, from the start: see `says_it_is_still_running`), which the
/// other tests needn't see among what they check.
fn answered_alive(stream: &TcpStream, request: &Request) -> bool {
    if !request.path.ends_with("/alive") {
        return false;
    }
    let _ = (&*stream).write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n");
    true
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
    logged_in(home.path(), port);
    let events = home.path().join("events");
    std::fs::create_dir_all(&events).unwrap();
    let file = events.join("x-s.jsonl");
    let first = line(1) + &line(2);
    std::fs::write(&file, &first).unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_agent-graph"))
        .args(["watch-remote", &format!("--url=http://127.0.0.1:{port}")])
        .env("AGENT_GRAPH_HOME", home.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    // The link comes first, straight away.
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut link = String::new();
    stdout.read_line(&mut link).unwrap();
    assert_eq!(link.trim(), "https://site.example/watch");

    let create = requests.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(create.path, "/api/logs");
    assert_eq!(create.body, first, "the existing log");
    assert_eq!(create.headers["x-agent-graph-source"], "watch");
    assert_eq!(
        create.headers["authorization"], "Bearer agt_test",
        "the login's token: it's the account's share"
    );
    assert!(!create.headers.contains_key("x-agent-graph-password"));

    // New lines follow, alone, at the right offset, with the key; those
    // that arrive close together, in one request (at most one every few
    // seconds).
    agent_graph::store::append(&file, line(3).as_bytes()).unwrap();
    std::thread::sleep(Duration::from_millis(1200));
    agent_graph::store::append(&file, line(4).as_bytes()).unwrap();
    let append = requests.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(
        append.path,
        format!("/api/logs/abc123def456/append?offset={}", first.len())
    );
    assert_eq!(append.headers["authorization"], "Bearer the-key");
    assert_eq!(append.body, format!("{}{}", line(3), line(4)));

    child.kill().unwrap();
    child.wait().unwrap();
    // The key's kept only with the share, for the next run, where only its
    // owner can read it.
    let with_key: Vec<_> = walk(home.path())
        .into_iter()
        .filter(|(_, text)| text.contains("the-key"))
        .map(|(path, _)| path)
        .collect();
    assert_eq!(with_key.len(), 1, "{with_key:?}");
    assert!(with_key[0].starts_with(home.path().join("shares")));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&with_key[0]), 0o600);
        assert_eq!(mode(&home.path().join("shares")), 0o700);
    }
}

fn walk(dir: &std::path::Path) -> Vec<(std::path::PathBuf, String)> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk(&path));
        } else {
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            out.push((path, text));
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
    logged_in(home.path(), port);
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
    logged_in(home.path(), port);
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
    logged_in(home.path(), port);
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
    logged_in(home.path(), port);
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
    logged_in(home.path(), port);
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
            if answered_alive(&stream, &request) {
                continue;
            }
            let reply = if request.path == "/api/logs" {
                let body = r#"{"id":"abc123def456","url":"https://site.example/watch","writeToken":"the-key","accountStatus":"active"}"#;
                if tx.send(request).is_err() {
                    return;
                }
                format!(
                    "HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
            } else {
                // A claim (carrying on with a share) is answered with /watch.
                let claim = request.path.ends_with("/watch");
                if tx.send(request).is_err() {
                    return;
                }
                let Ok(status) = reply_rx.recv() else { return };
                let body = if status == 200 && claim {
                    r#"{"url":"https://site.example/watch","accountStatus":"active"}"#
                } else {
                    ""
                };
                format!(
                    "HTTP/1.1 {status} Whatever\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
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
    logged_in(home.path(), port);
    let events = home.path().join("events");
    std::fs::create_dir_all(&events).unwrap();
    let file = events.join("x-s.jsonl");
    let first = line(1);
    std::fs::write(&file, &first).unwrap();
    let _running = Stopped(
        Command::new(env!("CARGO_BIN_EXE_agent-graph"))
            .args(["watch-remote", "--url"])
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
    logged_in(home.path(), port);
    let events = home.path().join("events");
    std::fs::create_dir_all(&events).unwrap();
    let file = events.join("x-s.jsonl");
    std::fs::write(&file, (0..2500).map(numbered).collect::<String>()).unwrap();
    let _running = Stopped(
        Command::new(env!("CARGO_BIN_EXE_agent-graph"))
            .args(["watch-remote", "--url"])
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
    logged_in(home.path(), port);
    let events = home.path().join("events");
    std::fs::create_dir_all(&events).unwrap();
    let file = events.join("x-s.jsonl");
    std::fs::write(&file, (0..2500).map(numbered).collect::<String>()).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_agent-graph"))
        .args(["watch-remote", "--url"])
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

/// R47: a redirect isn't followed. (One would carry the login's token on
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
    logged_in(home.path(), port);
    std::fs::create_dir_all(home.path().join("events")).unwrap();
    let err = watch(home.path(), port, &[], &[]);
    assert!(err.contains("302"), "{err}");
    assert!(
        followed.recv_timeout(Duration::from_millis(300)).is_err(),
        "followed the redirect"
    );
}

/// `watch-remote` in `home`, against the site on `port`, with `args`; and
/// the link it prints first, once it does (a run carrying on prints it once
/// the site has taken its first chunk).
fn share(home: &std::path::Path, port: u16, args: &[&str]) -> (Stopped, mpsc::Receiver<String>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_agent-graph"))
        .arg("watch-remote")
        .args(args)
        .arg("--url")
        .arg(format!("http://127.0.0.1:{port}"))
        .env("AGENT_GRAPH_HOME", home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut link = String::new();
        let _ = BufReader::new(stdout).read_line(&mut link);
        let _ = tx.send(link.trim().to_string());
    });
    (Stopped(child), rx)
}

/// The link a run printed. (It's printed once its share is saved.)
fn link_of(printed: &mpsc::Receiver<String>) -> String {
    printed.recv_timeout(Duration::from_secs(10)).unwrap()
}

/// The next request that isn't a trim (answered 204), answering it `status`.
fn next_request(
    requests: &mpsc::Receiver<Request>,
    replies: &mpsc::Sender<u16>,
    status: u16,
) -> Request {
    loop {
        let r = requests.recv_timeout(Duration::from_secs(10)).unwrap();
        if r.path == "/api/logs" {
            return r;
        }
        if r.path.contains("/trim") {
            replies.send(204).unwrap();
            continue;
        }
        // Carrying on: the share's still the account's.
        if r.path.ends_with("/watch") {
            replies.send(200).unwrap();
            continue;
        }
        replies.send(status).unwrap();
        return r;
    }
}

/// A second run carries on with the share the first made: the same link,
/// no new log, and not the log again, but one keyframe of it, which the site
/// starts again at, appended where the first left off, and what's before it
/// trimmed.
#[test]
fn a_second_run_carries_on_with_the_last_share() {
    let (port, requests, replies) = scripted_site();
    let home = tempfile::tempdir().unwrap();
    logged_in(home.path(), port);
    let events = home.path().join("events");
    std::fs::create_dir_all(&events).unwrap();
    let file = events.join("x-s.jsonl");
    let first = line(1) + &line(2);
    std::fs::write(&file, &first).unwrap();

    let (running, printed) = share(home.path(), port, &[]);
    let create = next_request(&requests, &replies, 204);
    assert_eq!(create.path, "/api/logs");
    assert_eq!(create.body, first);
    let link = link_of(&printed);
    drop(running);

    agent_graph::store::append(&file, line(3).as_bytes()).unwrap();
    let (_running, printed) = share(home.path(), port, &[]);
    let resumed = next_request(&requests, &replies, 204);
    assert_eq!(
        resumed.path,
        format!("/api/logs/abc123def456/append?offset={}", first.len()),
        "where the last run left off, not a new log"
    );
    assert_eq!(resumed.headers["authorization"], "Bearer the-key");
    assert!(
        resumed.body.contains(r#""type":"keyframe""#),
        "{}",
        resumed.body
    );
    assert!(
        resumed.body.contains(r#""restart":true"#),
        "readers start again there"
    );
    assert!(!resumed.body.contains(&line(1)), "not the log again");
    assert_eq!(link_of(&printed), link, "the same link");
    let trim = requests.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(
        trim.path,
        format!("/api/logs/abc123def456/trim?before={}", first.len()),
        "what's before it deleted"
    );
    replies.send(204).unwrap();

    // New lines follow it.
    agent_graph::store::append(&file, line(4).as_bytes()).unwrap();
    let next = next_request(&requests, &replies, 204);
    assert_eq!(
        next.path,
        format!(
            "/api/logs/abc123def456/append?offset={}",
            first.len() + resumed.body.len()
        )
    );
    assert_eq!(next.body, line(4));
}

/// A new share, not the last one: with --new; logged in as someone else; or
/// when the last is gone.
#[test]
fn a_new_share_when_asked_for_or_when_the_last_cant_go_on() {
    let (port, requests, replies) = scripted_site();
    let home = tempfile::tempdir().unwrap();
    logged_in(home.path(), port);
    let events = home.path().join("events");
    std::fs::create_dir_all(&events).unwrap();
    std::fs::write(events.join("x-s.jsonl"), line(1)).unwrap();

    // Each run is stopped once it has printed its link, by when its share
    // is saved.
    let (running, printed) = share(home.path(), port, &[]);
    assert_eq!(next_request(&requests, &replies, 204).path, "/api/logs");
    link_of(&printed);
    drop(running);

    let (running, printed) = share(home.path(), port, &["--new"]);
    assert_eq!(
        next_request(&requests, &replies, 204).path,
        "/api/logs",
        "--new"
    );
    link_of(&printed);
    drop(running);

    // Logged in as someone else: shares are per account.
    logged_in_as(home.path(), port, "someone-else@example.com");
    let (running, printed) = share(home.path(), port, &[]);
    let r = next_request(&requests, &replies, 204);
    assert_eq!(r.path, "/api/logs", "another account");
    link_of(&printed);
    drop(running);

    // The last share (that account's) has been deleted by the site.
    let (_running, _printed) = share(home.path(), port, &[]);
    let tried = requests.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(tried.path.ends_with("/watch"), "{}", tried.path);
    replies.send(410).unwrap();
    assert_eq!(
        next_request(&requests, &replies, 204).path,
        "/api/logs",
        "gone"
    );
}

/// While one run shares, another prints its link, and leaves it be.
#[test]
fn a_share_being_shared_now_is_left_to_its_run() {
    let (port, requests, replies) = scripted_site();
    let home = tempfile::tempdir().unwrap();
    logged_in(home.path(), port);
    let events = home.path().join("events");
    std::fs::create_dir_all(&events).unwrap();
    std::fs::write(events.join("x-s.jsonl"), line(1)).unwrap();

    let (_running, printed) = share(home.path(), port, &[]);
    assert_eq!(next_request(&requests, &replies, 204).path, "/api/logs");
    let link = link_of(&printed);

    let second = Command::new(env!("CARGO_BIN_EXE_agent-graph"))
        .args(["watch-remote", "--url"])
        .arg(format!("http://127.0.0.1:{port}"))
        .env("AGENT_GRAPH_HOME", home.path())
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(second.status.success());
    assert_eq!(String::from_utf8_lossy(&second.stdout).trim(), link);
    assert!(
        requests.recv_timeout(Duration::from_millis(500)).is_err(),
        "nothing sent"
    );
}

/// A GET of `url` (on this machine), and the reply's status line and body.
fn get(url: &str) -> (String, String) {
    let rest = url.strip_prefix("http://").unwrap();
    let (host, path) = rest.split_once('/').unwrap();
    let mut stream = TcpStream::connect(host).unwrap();
    write!(
        stream,
        "GET /{path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut reply = String::new();
    stream.read_to_string(&mut reply).unwrap();
    let (head, body) = reply.split_once("\r\n\r\n").unwrap();
    (head.lines().next().unwrap().to_string(), body.to_string())
}

/// Not logged in: it opens the site's login (here, only prints it), waits
/// for the browser to come back with a one-time code and its `state`, trades
/// the code, with the secret its challenge was made from, for a token, keeps
/// that (readable only by the user), and shares with it. A callback with
/// another `state` isn't the login.
#[test]
fn logging_in_through_the_browser() {
    let (port, requests) = mock_site();
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join("events")).unwrap();
    std::fs::write(home.path().join("events/x-s.jsonl"), line(1)).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_agent-graph"))
        .args(["watch-remote", "--url"])
        .arg(format!("http://127.0.0.1:{port}"))
        .env("AGENT_GRAPH_HOME", home.path())
        .env("AGENT_GRAPH_NO_BROWSER", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stderr = BufReader::new(child.stderr.take().unwrap());
    let _running = Stopped(child);
    let login = loop {
        let mut l = String::new();
        assert!(stderr.read_line(&mut l).unwrap() > 0, "no login address");
        if let Some(at) = l.find("http://127.0.0.1:") {
            if l.contains("/login?") {
                break l[at..].trim().to_string();
            }
        }
    };
    let query = login.split_once('?').unwrap().1;
    let param = |name: &str| {
        query
            .split('&')
            .find_map(|p| p.strip_prefix(&format!("{name}=")))
            .unwrap()
            .to_string()
    };
    let (cli, state, challenge) = (param("cli"), param("state"), param("challenge"));
    assert_eq!(state.len(), 43);

    // Not this login's state: refused, and it keeps waiting.
    let (status, _) = get(&format!(
        "http://127.0.0.1:{cli}/callback?code=c0de&state=someone-elses"
    ));
    assert!(status.contains("400"), "{status}");

    let (status, page) = get(&format!(
        "http://127.0.0.1:{cli}/callback?code=c0de&state={state}"
    ));
    assert!(status.contains("200"), "{status}");
    assert!(
        page.contains("me@example.com") && page.contains("close this window"),
        "{page}"
    );

    let trade = requests.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(trade.path, "/api/cli/token");
    let body: serde_json::Value = serde_json::from_str(&trade.body).unwrap();
    assert_eq!(body["code"], "c0de");
    let verifier = body["verifier"].as_str().unwrap();
    let digest = ring::digest::digest(&ring::digest::SHA256, verifier.as_bytes());
    assert_eq!(
        agent_graph::remote::base64url(digest.as_ref()),
        challenge,
        "the challenge is the verifier's SHA-256"
    );

    let create = requests.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(create.path, "/api/logs");
    assert_eq!(create.headers["authorization"], "Bearer agt_from_login");
    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.path().join("account.json")).unwrap())
            .unwrap();
    assert_eq!(saved["token"], "agt_from_login");
    assert_eq!(saved["email"], "me@example.com");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(home.path().join("account.json"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

/// `--logout`: the site forgets the login, and it's removed here.
#[test]
fn logging_out() {
    let (port, requests) = mock_site();
    let home = tempfile::tempdir().unwrap();
    logged_in(home.path(), port);
    let out = Command::new(env!("CARGO_BIN_EXE_agent-graph"))
        .args(["watch-remote", "--logout", "--url"])
        .arg(format!("http://127.0.0.1:{port}"))
        .env("AGENT_GRAPH_HOME", home.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("Logged out"));
    let told = requests.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(told.path, "/api/cli/logout");
    assert_eq!(told.headers["authorization"], "Bearer agt_test");
    assert!(!home.path().join("account.json").exists());
}

/// An account whose free days are over: the site refuses to start the share
/// (402) till it's subscribed, and later refuses an append (402) once its
/// time's up again; each time, watch-remote opens the account page, asks
/// till the account can share, then carries on.
#[test]
fn waits_for_the_account_to_subscribe() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, requests) = mpsc::channel();
    std::thread::spawn(move || {
        let (mut creates, mut appends) = (0, 0);
        for stream in listener.incoming().flatten() {
            let request = read_request(&stream);
            if answered_alive(&stream, &request) {
                continue;
            }
            let reply = |status: &str, body: &str| {
                format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
            };
            let unpaid = || reply("402 Payment Required", "Subscribe, at the account page");
            let text = if request.path == "/api/logs" {
                creates += 1;
                if creates == 1 {
                    unpaid()
                } else {
                    reply(
                        "201 Created",
                        r#"{"id":"abc123def456","url":"https://site.example/watch","writeToken":"the-key","accountStatus":"active","freeUntil":null}"#,
                    )
                }
            } else if request.path == "/api/cli/account" {
                reply(
                    "200 OK",
                    r#"{"email":"me@example.com","accountStatus":"active","freeUntil":null,"canShare":true}"#,
                )
            } else if request.path.ends_with("/watch") {
                reply(
                    "200 OK",
                    r#"{"url":"https://site.example/watch","accountStatus":"active","freeUntil":null}"#,
                )
            } else {
                appends += 1;
                if appends == 1 {
                    unpaid()
                } else {
                    "HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n".to_string()
                }
            };
            (&stream).write_all(text.as_bytes()).unwrap();
            if tx.send(request).is_err() {
                return;
            }
        }
    });

    let home = tempfile::tempdir().unwrap();
    logged_in(home.path(), port);
    let events = home.path().join("events");
    std::fs::create_dir_all(&events).unwrap();
    let file = events.join("x-s.jsonl");
    std::fs::write(&file, line(1)).unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_agent-graph"))
        .args(["watch-remote", &format!("--url=http://127.0.0.1:{port}")])
        .env("AGENT_GRAPH_HOME", home.path())
        .env("AGENT_GRAPH_NO_BROWSER", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stderr = child.stderr.take().unwrap();
    let said = std::thread::spawn(move || {
        let mut all = String::new();
        BufReader::new(stderr).read_to_string(&mut all).ok();
        all
    });

    let next = || requests.recv_timeout(Duration::from_secs(15)).unwrap().path;
    assert_eq!(next(), "/api/logs", "refused: it has to subscribe");
    assert_eq!(next(), "/api/cli/account", "asked till it has");
    assert_eq!(next(), "/api/logs", "then started");

    let mut log = std::fs::OpenOptions::new()
        .append(true)
        .open(&file)
        .unwrap();
    log.write_all(line(2).as_bytes()).unwrap();
    assert!(
        next().starts_with("/api/logs/abc123def456/append"),
        "refused: time's up"
    );
    assert_eq!(next(), "/api/cli/account", "asked till it's paid");
    assert_eq!(next(), "/api/logs/abc123def456/watch", "started again");
    assert!(
        next().starts_with("/api/logs/abc123def456/append"),
        "and carried on"
    );

    child.kill().unwrap();
    child.wait().unwrap();
    let said = said.join().unwrap();
    assert_eq!(
        said.matches("Opening your account page").count(),
        2,
        "{said}"
    );
    assert!(
        said.contains(&format!("http://127.0.0.1:{port}/account")),
        "{said}"
    );
}

/// Every minute or so, from the start, it tells the site it's still
/// running, with the log's key, on which computer, and how many sessions
/// it's watching, and their summaries: those with an event in the last day
/// (a session quiet longer isn't counted).
#[test]
fn says_it_is_still_running() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, alive) = mpsc::channel();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let request = read_request(&stream);
            let reply = if request.path == "/api/logs" {
                let body = r#"{"id":"abc123def456","url":"https://site.example/watch","writeToken":"the-key","accountStatus":"active"}"#;
                format!(
                    "HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
            } else {
                "HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n".to_string()
            };
            (&stream).write_all(reply.as_bytes()).unwrap();
            if request.path.ends_with("/alive") && tx.send(request).is_err() {
                return;
            }
        }
    });
    let home = tempfile::tempdir().unwrap();
    logged_in(home.path(), port);
    let events = home.path().join("events");
    std::fs::create_dir_all(&events).unwrap();
    std::fs::write(events.join("x-s.jsonl"), line(1)).unwrap();
    let quiet = events.join("x-t.jsonl");
    std::fs::write(&quiet, line(2).replace("x:s", "x:t")).unwrap();
    std::fs::File::options()
        .write(true)
        .open(&quiet)
        .unwrap()
        .set_modified(std::time::SystemTime::now() - Duration::from_secs(2 * 24 * 60 * 60))
        .unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_agent-graph"))
        .args(["watch-remote", &format!("--url=http://127.0.0.1:{port}")])
        .env("AGENT_GRAPH_HOME", home.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let said = alive.recv_timeout(Duration::from_secs(10));
    child.kill().unwrap();
    child.wait().unwrap();
    let said = said.unwrap();
    assert_eq!(said.path, "/api/logs/abc123def456/alive");
    assert_eq!(said.headers["authorization"], "Bearer the-key");
    let body: serde_json::Value = serde_json::from_str(&said.body).unwrap();
    assert_eq!(body["sessions"], 1);
    assert!(body["host"].is_string(), "the computer it's on");
    // Its summaries, as the viewer's list shows them: the one it's watching.
    let summary = body["summary"].as_object().unwrap();
    assert_eq!(
        summary.keys().collect::<Vec<_>>(),
        ["x:s"],
        "not the quiet one"
    );
    assert_eq!(summary["x:s"]["state"], "working");
}

/// Where `--autostart` puts its service, under `home` (as $HOME).
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn service_file(home: &std::path::Path) -> std::path::PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library/LaunchAgents/com.chofter.agent-graph.watch-remote.plist")
    } else {
        home.join(".config/systemd/user/agent-graph-watch-remote.service")
    }
}

/// `--autostart` writes the service that runs it in the background at
/// login, for someone logged in; `--no-autostart` removes it. (Only the
/// files: launchd and systemd aren't told, in a test.)
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn autostart_sets_up_a_service_and_no_autostart_removes_it() {
    let home = tempfile::tempdir().unwrap();
    let data = home.path().join(".agent-graph");
    std::fs::create_dir_all(&data).unwrap();
    let site = "https://sharing.example";
    let account = serde_json::json!({"site": site, "token": "agt_test", "email": "me@example.com"});
    std::fs::write(data.join("account.json"), account.to_string()).unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_agent-graph"))
            .args(args)
            .env("HOME", home.path())
            .env("AGENT_GRAPH_HOME", &data)
            .env("AGENT_GRAPH_NO_SERVICE_MANAGER", "1")
            .env("AGENT_GRAPH_NO_BROWSER", "1")
            .env_remove("XDG_CONFIG_HOME")
            .output()
            .unwrap()
    };

    let out = run(&["watch-remote", "--autostart", &format!("--url={site}")]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(said.contains("whenever you log in"), "{said}");
    assert!(said.contains("me@example.com"), "{said}");
    let service = std::fs::read_to_string(service_file(home.path())).unwrap();
    assert!(service.contains("watch-remote"), "{service}");
    assert!(service.contains("--background"), "{service}");
    assert!(service.contains(&format!("--url={site}")), "{service}");
    assert!(
        service.contains(&data.to_string_lossy().to_string()),
        "its data directory: {service}"
    );

    // With another option that doesn't go with it, it's refused.
    assert!(
        !run(&["watch-remote", "--autostart", "--new"])
            .status
            .success()
    );

    let out = run(&["watch-remote", "--no-autostart"]);
    assert!(out.status.success());
    assert!(!service_file(home.path()).exists());
    let out = run(&["watch-remote", "--no-autostart"]);
    assert!(String::from_utf8_lossy(&out.stdout).contains("wasn't set to start"));
}

/// The user's Run key's entry, in `key` (a test's own, under
/// HKEY_CURRENT_USER: never the real one), as `reg query` shows it.
#[cfg(windows)]
fn run_entry(key: &str) -> Option<String> {
    let out = Command::new("reg")
        .args([
            "query",
            &format!(r"HKCU\{key}"),
            "/v",
            "AgentGraphWatchRemote",
        ])
        .output()
        .unwrap();
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// On Windows, `--autostart` adds an entry to the Run key (the Startup
/// apps), which runs it `--autostarted` at login, and `--no-autostart`
/// removes it. (In a key of the test's own, and nothing's started.)
#[cfg(windows)]
#[test]
fn autostart_adds_a_startup_app_and_no_autostart_removes_it() {
    let home = tempfile::tempdir().unwrap();
    let data = home.path().join("agent graph");
    std::fs::create_dir_all(&data).unwrap();
    let site = "https://sharing.example";
    let account = serde_json::json!({"site": site, "token": "agt_test", "email": "me@example.com"});
    std::fs::write(data.join("account.json"), account.to_string()).unwrap();
    let key = format!(
        r"Software\agent-graph-tests\{}",
        home.path().file_name().unwrap().to_string_lossy()
    );
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_agent-graph"))
            .args(args)
            .env("AGENT_GRAPH_HOME", &data)
            .env("AGENT_GRAPH_RUN_KEY", &key)
            .env("AGENT_GRAPH_NO_SERVICE_MANAGER", "1")
            .env("AGENT_GRAPH_NO_BROWSER", "1")
            .output()
            .unwrap()
    };

    let out = run(&["watch-remote", "--autostart", &format!("--url={site}")]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(said.contains("whenever you log in"), "{said}");
    assert!(said.contains("Startup apps"), "{said}");
    let entry = run_entry(&key).expect("the entry");
    assert!(entry.contains("agent-graph"), "{entry}");
    assert!(
        entry.contains("\"watch-remote\" \"--autostarted\""),
        "{entry}"
    );
    assert!(entry.contains(&format!("\"--url={site}\"")), "{entry}");
    assert!(
        entry.contains(&format!("\"--data-dir={}\"", data.display())),
        "its data directory: {entry}"
    );

    let out = run(&["watch-remote", "--no-autostart"]);
    assert!(out.status.success());
    assert!(run_entry(&key).is_none());
    let out = run(&["watch-remote", "--no-autostart"]);
    assert!(String::from_utf8_lossy(&out.stdout).contains("wasn't set to start"));
    let _ = Command::new("reg")
        .args(["delete", r"HKCU\Software\agent-graph-tests", "/f"])
        .output();
}

/// Run at login on Windows (`--autostarted`), it starts a copy of itself
/// without a window, and stops at once; that copy runs `watch-remote
/// --background`, its output in the log, while its process is in the pid
/// file. Not logged in, that stops by itself (not a failure), so the copy
/// stops too, rather than starting it again.
#[cfg(windows)]
#[test]
fn started_at_login_on_windows_it_runs_watch_remote_without_a_window() {
    let (port, _requests) = mock_site();
    let data = tempfile::tempdir().unwrap();
    let started = std::time::Instant::now();
    let out = Command::new(env!("CARGO_BIN_EXE_agent-graph"))
        .args([
            "watch-remote",
            "--autostarted",
            &format!("--url=http://127.0.0.1:{port}"),
        ])
        .arg(format!("--data-dir={}", data.path().display()))
        .env_remove("AGENT_GRAPH_HOME")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "it doesn't wait"
    );

    let log = data.path().join("watch-remote.log");
    let pid_file = data.path().join("watch-remote.pid");
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    let mut said = String::new();
    while std::time::Instant::now() < deadline {
        said = std::fs::read_to_string(&log).unwrap_or_default();
        if said.contains("Not logged in") && !pid_file.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(said.contains("Not logged in"), "the log: {said}");
    assert!(!said.contains("starting it again"), "the log: {said}");
    assert!(!pid_file.exists(), "it stopped");
}

/// Started at login but not logged in, it doesn't open a browser or fail
/// (which would have it started again and again): it says how to log in,
/// and stops.
#[test]
fn a_background_run_that_needs_a_login_says_so_and_stops() {
    let (port, requests) = mock_site();
    let home = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_agent-graph"))
        .args([
            "watch-remote",
            "--background",
            &format!("--url=http://127.0.0.1:{port}"),
        ])
        .env("AGENT_GRAPH_HOME", home.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(said.contains("Not logged in"), "{said}");
    assert!(said.contains("agent-graph watch-remote --url="), "{said}");
    assert!(
        requests.recv_timeout(Duration::from_millis(300)).is_err(),
        "nothing's sent to the site"
    );
}

/// A site that knows one API token (`agt_known`, for me@example.com), makes
/// shares, and sends on each request.
fn token_site() -> (u16, mpsc::Receiver<Request>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let request = read_request(&stream);
            if answered_alive(&stream, &request) {
                continue;
            }
            let json = |status: &str, body: &str| {
                format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
            };
            let known = request.headers.get("authorization").map(String::as_str)
                == Some("Bearer agt_known");
            let reply = if request.path == "/api/cli/account" {
                if known {
                    json(
                        "200 OK",
                        r#"{"email":"me@example.com","accountStatus":"active","canShare":true}"#,
                    )
                } else {
                    json("401 Unauthorized", "")
                }
            } else if request.path == "/api/logs" {
                json(
                    "201 Created",
                    r#"{"id":"abc123def456","url":"https://site.example/watch","writeToken":"the-key","accountStatus":"active"}"#,
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

/// With an API token in AGENT_GRAPH_TOKEN, and no login yet, it shares with
/// that, without the browser, and keeps it as the login for next time.
#[test]
fn an_api_token_in_the_environment_is_the_login() {
    let (port, requests) = token_site();
    let home = tempfile::tempdir().unwrap();
    let events = home.path().join("events");
    std::fs::create_dir_all(&events).unwrap();
    std::fs::write(events.join("x-s.jsonl"), line(1)).unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_agent-graph"))
        .args(["watch-remote", &format!("--url=http://127.0.0.1:{port}")])
        .env("AGENT_GRAPH_HOME", home.path())
        .env("AGENT_GRAPH_TOKEN", "agt_known")
        .env("AGENT_GRAPH_NO_BROWSER", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let create = loop {
        let r = requests.recv_timeout(Duration::from_secs(10)).unwrap();
        if r.path == "/api/logs" {
            break r;
        }
        assert_eq!(r.path, "/api/cli/account", "only asked whose it is first");
    };
    child.kill().unwrap();
    child.wait().unwrap();
    assert_eq!(create.headers["authorization"], "Bearer agt_known");
    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.path().join("account.json")).unwrap())
            .unwrap();
    assert_eq!(saved["token"], "agt_known");
    assert_eq!(saved["email"], "me@example.com");
}

/// A token the site doesn't know (revoked, say) is said so; in the
/// background it stops without failing, so it isn't started again and
/// again, and nothing's shared.
#[test]
fn an_api_token_the_site_doesnt_know_is_refused() {
    let (port, requests) = token_site();
    let home = tempfile::tempdir().unwrap();
    let run = |background: bool| {
        let mut args = vec![
            "watch-remote".to_string(),
            format!("--url=http://127.0.0.1:{port}"),
        ];
        if background {
            args.push("--background".into());
        }
        Command::new(env!("CARGO_BIN_EXE_agent-graph"))
            .args(&args)
            .env("AGENT_GRAPH_HOME", home.path())
            .env("AGENT_GRAPH_TOKEN", "agt_revoked")
            .env("AGENT_GRAPH_NO_BROWSER", "1")
            .output()
            .unwrap()
    };
    let out = run(false);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("AGENT_GRAPH_TOKEN isn't a login"));
    let out = run(true);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("revoked"));
    while let Ok(r) = requests.recv_timeout(Duration::from_millis(300)) {
        assert_ne!(r.path, "/api/logs", "nothing's shared");
    }
    assert!(
        !home.path().join("account.json").exists(),
        "and it isn't kept"
    );
}

/// A site that answers pings, and knows one login, `agt_good`.
fn pinged_site() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let request = read_request(&stream);
            let reply = |status: &str, body: &str| {
                format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
            };
            let reply = if let Some(n) = request.path.strip_prefix("/api/ping?n=") {
                reply("200 OK", n)
            } else if request.path == "/api/cli/account" {
                match request.headers.get("authorization").map(String::as_str) {
                    Some("Bearer agt_good") => reply("200 OK", r#"{"email":"me@example.com"}"#),
                    _ => reply("401 Unauthorized", r#"{"error":"unknown"}"#),
                }
            } else {
                reply("404 Not Found", "")
            };
            let _ = (&stream).write_all(reply.as_bytes());
        }
    });
    port
}

/// `watch-remote --check`: ✓ or ✗ for reaching the site and the token, and
/// a failure unless both are in place.
#[test]
fn check_says_whether_the_site_and_the_token_are_in_place() {
    let home = tempfile::tempdir().unwrap();
    let port = pinged_site();
    let check = |token: Option<&str>, url: String| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_agent-graph"));
        command
            .args(["watch-remote", "--check", "--url", &url])
            .env("AGENT_GRAPH_HOME", home.path())
            .env_remove("AGENT_GRAPH_TOKEN");
        if let Some(token) = token {
            command.env("AGENT_GRAPH_TOKEN", token);
        }
        let out = command.output().unwrap();
        let said = String::from_utf8_lossy(&out.stdout).into_owned()
            + &String::from_utf8_lossy(&out.stderr);
        (out.status.success(), said)
    };
    let site = format!("http://127.0.0.1:{port}");

    let (ok, said) = check(Some("agt_good"), site.clone());
    assert!(ok, "{said}");
    assert!(
        said.contains("✓ 127.0.0.1:") && said.contains("can be reached"),
        "{said}"
    );
    assert!(
        said.contains("✓ AGENT_GRAPH_TOKEN is a valid login, for me@example.com"),
        "{said}"
    );

    let (ok, said) = check(Some("agt_revoked"), site.clone());
    assert!(!ok);
    assert!(said.contains("✗ AGENT_GRAPH_TOKEN isn't a login"), "{said}");

    let (ok, said) = check(None, site);
    assert!(!ok);
    assert!(said.contains("✗ AGENT_GRAPH_TOKEN isn't set"), "{said}");
    assert!(said.contains("not its secrets"), "{said}");

    // Nothing listening: it can't be reached, nor the token checked.
    let closed = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let (ok, said) = check(Some("agt_good"), format!("http://127.0.0.1:{closed}"));
    assert!(!ok);
    assert!(said.contains("can't be reached"), "{said}");
    assert!(said.contains("can't be checked"), "{said}");
}
