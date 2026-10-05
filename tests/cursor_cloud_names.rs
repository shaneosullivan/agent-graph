//! In Cursor's cloud, with a Cursor API key, its agents' names come from
//! Cursor's API (`adapter::cursor::name_cloud_chats`), here a stand-in.
//! A test binary of its own: it sets the environment.

use std::io::{Read, Write};
use std::net::TcpListener;

use agent_graph::adapter::cursor;

const CHAT: &str = "bc-7a1c2e9f-3b4d-4e5f-8a6b-1c2d3e4f5a6b";

/// A stand-in for Cursor's API: each request answered with `name` for the
/// key "good-key", and 401 for any other. Returns its address, and what it
/// was asked.
fn api(name: &'static str) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = format!("http://{}", listener.local_addr().unwrap());
    let asked = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let log = asked.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut req = Vec::new();
            let mut buf = [0u8; 512];
            while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                match stream.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => req.extend_from_slice(&buf[..n]),
                }
            }
            let req = String::from_utf8_lossy(&req).into_owned();
            log.lock()
                .unwrap()
                .push(req.lines().next().unwrap_or("").to_string());
            let good = req.contains("Authorization: Bearer good-key")
                || req.contains("authorization: Bearer good-key");
            let (status, body) = if good {
                (
                    "200 OK",
                    format!(r#"{{"id":"{CHAT}","name":"{name}","status":"ACTIVE"}}"#),
                )
            } else {
                ("401 Unauthorized", "{}".to_string())
            };
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    (addr, asked)
}

#[test]
fn a_cloud_agents_name_comes_from_cursors_api() {
    let home = tempfile::tempdir().unwrap();
    let events = home.path().join("events");
    std::fs::create_dir_all(&events).unwrap();
    let file = events.join(format!("cursor-{CHAT}.jsonl"));
    std::fs::write(
        &file,
        format!(
            "{{\"v\":1,\"id\":\"01K0000000000000000000001\",\"ts\":\"2026-10-05T09:00:00.000Z\",\"type\":\"session.started\",\"node\":\"cursor:{CHAT}\",\"data\":{{\"cwd\":\"github.com/acme/shop\"}}}}\n\
             {{\"v\":1,\"id\":\"01K0000000000000000000002\",\"ts\":\"2026-10-05T09:00:01.000Z\",\"type\":\"status\",\"node\":\"cursor:{CHAT}\",\"data\":{{\"state\":\"working\"}}}}\n"
        ),
    )
    .unwrap();
    let (addr, asked) = api("Repository survey diagnostics");
    // SAFETY: this test binary has this one test, so nothing else reads the
    // environment meanwhile.
    unsafe {
        std::env::set_var("AGENT_GRAPH_HOME", home.path());
        std::env::set_var("CLOUD_AGENT_ALL_SECRET_NAMES", "CURSOR_API_KEY");
        std::env::set_var("AGENT_GRAPH_CURSOR_API", &addr);
        std::env::set_var("NO_PROXY", "*");
        std::env::set_var("CURSOR_API_KEY", "wrong-key");
        // No metadata socket: a machine of one's own, say.
        std::env::set_var("CURSOR_AGENT_SOCKET", home.path().join("none.sock"));
    }
    // A key Cursor refuses: no name.
    assert_eq!(cursor::name_cloud_chats(&events), 0);
    assert_eq!(cursor::title_of(CHAT), None);
    assert_eq!(cursor::api_key_works("wrong-key"), Some(false));

    // (Asked again only after a minute: a new chat, for the right key.)
    let other = CHAT.replace("7a1c", "8b2d");
    std::fs::copy(&file, events.join(format!("cursor-{other}.jsonl"))).unwrap();
    let text = std::fs::read_to_string(&file)
        .unwrap()
        .replace(CHAT, &other);
    std::fs::write(events.join(format!("cursor-{other}.jsonl")), text).unwrap();
    unsafe { std::env::set_var("CURSOR_API_KEY", "good-key") };
    assert_eq!(
        cursor::name_cloud_chats(&events),
        1,
        "{:?}",
        asked.lock().unwrap()
    );
    assert_eq!(
        cursor::title_of(&other).as_deref(),
        Some("Repository survey diagnostics")
    );
    assert_eq!(cursor::api_key_works("good-key"), Some(true));
    // Said at once, with the chat's state as it stood.
    let text = std::fs::read_to_string(events.join(format!("cursor-{other}.jsonl"))).unwrap();
    let last: serde_json::Value = serde_json::from_str(text.lines().last().unwrap()).unwrap();
    assert_eq!(last["type"], "status");
    assert_eq!(last["data"]["state"], "working");
    assert_eq!(last["data"]["title"], "Repository survey diagnostics");
    // Not asked again within the minute.
    let before = asked.lock().unwrap().len();
    assert_eq!(cursor::name_cloud_chats(&events), 0);
    assert!(
        asked.lock().unwrap().len() == before,
        "asked again too soon"
    );
    // The key never goes anywhere but Cursor's API, and only as a header.
    assert!(
        asked
            .lock()
            .unwrap()
            .iter()
            .all(|l| !l.contains("good-key"))
    );

    // Cursor's own cloud machines say it themselves, on their metadata
    // socket: no key, and at once.
    #[cfg(unix)]
    {
        let third = CHAT.replace("7a1c", "9c3e");
        let socket = home.path().join("api.sock");
        metadata_socket(&socket, third.clone(), "Agent-graph system status");
        let text = std::fs::read_to_string(&file)
            .unwrap()
            .replace(CHAT, &third);
        std::fs::write(events.join(format!("cursor-{third}.jsonl")), text).unwrap();
        unsafe {
            std::env::set_var("CURSOR_AGENT_SOCKET", &socket);
            std::env::remove_var("CURSOR_API_KEY");
        }
        assert_eq!(
            cursor::metadata("agent/id").as_deref(),
            Some(third.as_str())
        );
        assert_eq!(
            cursor::title_of(&third).as_deref(),
            Some("Agent-graph system status")
        );
        assert_eq!(cursor::name_cloud_chats(&events), 1);
        let text = std::fs::read_to_string(events.join(format!("cursor-{third}.jsonl"))).unwrap();
        assert!(
            text.contains("\"title\":\"Agent-graph system status\""),
            "{text}"
        );
        // Another conversation on that machine (a subagent's) isn't its agent.
        assert_eq!(cursor::title_of(CHAT), None);
        // A path it has no value for.
        assert_eq!(cursor::metadata("agent/nothing"), None);
    }
}

/// A stand-in for Cursor's Agent Metadata Socket at `path`, for agent `id`
/// named `name`.
#[cfg(unix)]
fn metadata_socket(path: &std::path::Path, id: String, name: &'static str) {
    let listener = std::os::unix::net::UnixListener::bind(path).unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            // The whole request, however it's sent.
            let mut req = Vec::new();
            let mut buf = [0u8; 512];
            while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                match stream.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => req.extend_from_slice(&buf[..n]),
                }
            }
            let req = String::from_utf8_lossy(&req).into_owned();
            let path = req.split_whitespace().nth(1).unwrap_or("");
            let body = match path {
                "/v1/meta-data/agent/id" => Some(id.clone()),
                "/v1/meta-data/agent/name" => Some(format!("{name}\n")),
                "/v1/meta-data/workspace/repo-url" => Some("github.com/acme/shop".to_string()),
                _ => None,
            };
            let _ = match body {
                Some(b) => write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{b}",
                    b.len()
                ),
                None => write!(
                    stream,
                    "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n"
                ),
            };
        }
    });
}
