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
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]).into_owned();
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
}
