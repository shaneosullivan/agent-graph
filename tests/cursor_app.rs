//! What Cursor's app saves as it waits on you (a tool call to approve, a
//! question), written into its chats' logs, as the viewer and `watch-remote`
//! do so the site shows it. Its own test binary: it reads the app's
//! database from `HOME`, which it sets for the whole process.

use std::path::Path;

const CHAT: &str = "7531965f-3342-4502-bb40-000000000000";

fn line(kind: &str, data: serde_json::Value, adapter: &str) -> String {
    serde_json::json!({"v": 1, "id": "01M4403YPHSAHJDETT768QW1SF", "ts": "2026-10-04T21:55:00.000Z",
        "type": kind, "node": format!("cursor:{CHAT}"),
        "source": {"provider": "cursor", "adapter": adapter}, "data": data})
    .to_string()
        + "\n"
}

fn last(file: &Path) -> serde_json::Value {
    let text = std::fs::read_to_string(file).unwrap();
    serde_json::from_str(text.lines().last().unwrap()).unwrap()
}

#[test]
fn waits_the_app_saves_are_written_into_the_chats_log_as_they_start_and_stop() {
    let home = tempfile::tempdir().unwrap();
    // SAFETY: this test binary has this one test, so nothing else reads the
    // environment meanwhile.
    unsafe { std::env::set_var("HOME", home.path()) };
    let store = agent_graph::adapter::cursor::app_store(home.path()).unwrap();
    std::fs::create_dir_all(store.parent().unwrap()).unwrap();
    let db = rusqlite::Connection::open(&store).unwrap();
    db.execute_batch(&format!(
        "create table composerHeaders (composerId text primary key, value text); \
         create table cursorDiskKV (key text primary key, value blob); \
         insert into composerHeaders values ('{CHAT}', '{{}}');"
    ))
    .unwrap();
    let ask = |review: &str| {
        let tool = serde_json::json!({"type": 2, "toolFormerData": {"name": "run_terminal_command_v2",
            "status": "loading", "additionalData": {"reviewData": {"status": review}}}});
        db.execute(
            "insert or replace into cursorDiskKV values (?1, ?2)",
            [format!("bubbleId:{CHAT}:m1"), tool.to_string()],
        )
        .unwrap();
    };
    let events = home.path().join("ag/events");
    std::fs::create_dir_all(&events).unwrap();
    let file = events.join(format!("cursor-{CHAT}.jsonl"));
    std::fs::write(
        &file,
        line("session.started", serde_json::json!({}), "cursor@1")
            + &line(
                "status",
                serde_json::json!({"state": "working"}),
                "cursor@1",
            )
            + &line(
                "activity",
                serde_json::json!({"tool": "Shell", "label": "command 1", "may_ask": true}),
                "cursor@1",
            ),
    )
    .unwrap();
    use agent_graph::adapter::cursor::mark_waiting;

    // Asking: marked once.
    ask("Requested");
    assert_eq!(mark_waiting(&events), 1);
    assert_eq!(last(&file)["data"]["state"], "input_required");
    assert_eq!(last(&file)["data"]["summary"], "Waiting for your approval");
    assert_eq!(last(&file)["source"]["adapter"], "cursor-app@1");
    assert_eq!(mark_waiting(&events), 0, "said once");
    // Approved: working again.
    ask("Done");
    assert_eq!(mark_waiting(&events), 1);
    assert_eq!(last(&file)["data"]["state"], "working");
    assert_eq!(mark_waiting(&events), 0);
    // A turn that ended while asking isn't asking.
    std::fs::write(
        &file,
        std::fs::read_to_string(&file).unwrap()
            + &line("status", serde_json::json!({"state": "idle"}), "cursor@1"),
    )
    .unwrap();
    ask("Requested");
    assert_eq!(mark_waiting(&events), 0);
}
