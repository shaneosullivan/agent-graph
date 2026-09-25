//! Reducer behaviour that doesn't depend on any one provider.

mod common;

use std::time::{Duration, SystemTime};

use agent_graph::event::{Envelope, State};
use agent_graph::reducer::{self, Options};
use common::t0;
use serde_json::{Value, json};

fn ev(secs: u64, node: &str, kind: &str, data: Value) -> Envelope {
    let at = t0() + Duration::from_secs(secs);
    Envelope {
        v: 1,
        id: ulid::Ulid::from_datetime(at).to_string(),
        ts: humantime::format_rfc3339_millis(at).to_string(),
        kind: kind.into(),
        node: node.into(),
        parent: None,
        source: None,
        trace: None,
        data,
    }
}

fn reduce_at(events: Vec<Envelope>, now_secs: u64) -> reducer::Graph {
    reducer::reduce(
        events,
        &Options {
            now: t0() + Duration::from_secs(now_secs),
            stale_after: Duration::from_secs(600),
        },
    )
}

fn tasks(n: usize) -> Value {
    let items: Vec<Value> = (0..n)
        .map(|i| json!({"id": i.to_string(), "text": format!("t{i}"), "status": "pending"}))
        .collect();
    json!({ "items": items })
}

#[test]
fn work_remaining_follows_waits_transitively() {
    // a waits on b; b has a subagent c and waits on d. Work before a can
    // continue is everything open in b, c and d.
    let events = vec![
        ev(0, "x:a", "session.started", json!({})),
        ev(1, "x:b", "tasks.updated", tasks(2)),
        ev(2, "x:b/c", "tasks.updated", tasks(1)),
        ev(3, "x:d", "tasks.updated", tasks(3)),
        ev(
            4,
            "x:a",
            "wait.started",
            json!({"wait_id": "w1", "on": "x:b"}),
        ),
        ev(
            5,
            "x:b",
            "wait.started",
            json!({"wait_id": "w2", "on": "x:d"}),
        ),
    ];
    let g = reduce_at(events, 10);
    let blocked = g.nodes["x:a"].blocked.as_ref().expect("a is blocked");
    assert_eq!(blocked.on, vec!["x:b".to_string()]);
    assert_eq!(blocked.nodes, 3);
    assert_eq!(blocked.open_tasks, 6);
}

#[test]
fn wait_cycles_terminate() {
    let events = vec![
        ev(
            0,
            "x:a",
            "wait.started",
            json!({"wait_id": "1", "on": "x:b"}),
        ),
        ev(
            1,
            "x:b",
            "wait.started",
            json!({"wait_id": "2", "on": "x:a"}),
        ),
    ];
    let g = reduce_at(events, 10);
    assert_eq!(g.nodes["x:a"].blocked.as_ref().unwrap().nodes, 1);
}

#[test]
fn wait_ends_when_the_target_finishes() {
    let events = vec![
        ev(
            0,
            "x:a",
            "wait.started",
            json!({"wait_id": "1", "on": "x:b"}),
        ),
        ev(1, "x:b", "session.ended", json!({})),
    ];
    let g = reduce_at(events, 10);
    assert!(g.nodes["x:a"].blocked.is_none());
}

#[test]
fn events_are_applied_in_time_order_whatever_the_file_order() {
    let events = vec![
        ev(5, "x:a", "status", json!({"state": "idle"})),
        ev(1, "x:a", "status", json!({"state": "working"})),
    ];
    assert_eq!(reduce_at(events, 10).nodes["x:a"].state, State::Idle);
}

#[test]
fn quiet_unfinished_nodes_are_stale() {
    let events = vec![
        ev(0, "x:done", "session.ended", json!({})),
        ev(0, "x:busy", "status", json!({"state": "working"})),
    ];
    let g = reduce_at(events, 3600);
    assert!(g.nodes["x:busy"].stale);
    assert!(!g.nodes["x:done"].stale, "finished nodes are never stale");
}

#[test]
fn session_end_cancels_running_agents() {
    let events = vec![
        ev(0, "x:s/a", "agent.spawned", json!({})),
        ev(1, "x:s/b", "agent.spawned", json!({})),
        ev(2, "x:s/b", "agent.finished", json!({"status": "completed"})),
        ev(3, "x:s", "session.ended", json!({})),
    ];
    let g = reduce_at(events, 10);
    assert_eq!(g.nodes["x:s/a"].state, State::Canceled);
    assert_eq!(g.nodes["x:s/b"].state, State::Completed);
}

#[test]
fn resumed_session_comes_back_but_compaction_does_not_reset_state() {
    let events = vec![
        ev(0, "x:s", "status", json!({"state": "working"})),
        ev(1, "x:s", "session.started", json!({"source": "compact"})),
    ];
    assert_eq!(reduce_at(events, 10).nodes["x:s"].state, State::Working);

    let events = vec![
        ev(0, "x:s", "session.ended", json!({})),
        ev(1, "x:s", "session.started", json!({"source": "resume"})),
    ];
    assert_eq!(reduce_at(events, 10).nodes["x:s"].state, State::Idle);
}

#[test]
fn unknown_types_and_bad_data_are_ignored() {
    let events = vec![
        ev(0, "x:s", "status", json!({"state": "working"})),
        ev(1, "x:s", "some.future.event", json!({"anything": 1})),
        ev(2, "x:s", "status", json!({"state": "not-a-state"})),
    ];
    let g = reduce_at(events, 10);
    assert_eq!(g.nodes["x:s"].state, State::Working);
}

#[test]
fn nodes_seen_only_as_agents_get_placeholder_parents() {
    let g = reduce_at(
        vec![ev(0, "x:s/a", "status", json!({"state": "working"}))],
        10,
    );
    assert_eq!(g.roots, vec!["x:s".to_string()]);
    assert_eq!(g.nodes["x:s"].children, vec!["x:s/a".to_string()]);
}

#[test]
fn default_options_use_now() {
    let opts = Options::default();
    assert!(opts.now <= SystemTime::now());
}
