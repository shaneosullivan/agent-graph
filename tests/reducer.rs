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

/// `sessions` sessions, each starting `agents` agents that it waits on,
/// three agents in turn that it guesses wrong at first (each guessed as the
/// oldest request left, g…-0, then named as its own when it returns), and
/// three sessions it starts from its shell; ending once they've finished. Every way the reducer pairs children with
/// requests, and every way it closes waits.
fn history(sessions: usize, agents: usize) -> Vec<Envelope> {
    let mut events = Vec::new();
    for s in 0..sessions {
        let session = format!("x:s{s}");
        let at = 11 * s as u64;
        events.push(ev(at, &session, "session.started", json!({})));
        for a in 0..agents {
            let agent = format!("{session}/a{a}");
            let call = format!("c{s}-{a}");
            events.push(ev(
                at + 1,
                &session,
                "spawn.requested",
                json!({"call_id": call, "kind": "agent"}),
            ));
            events.push(ev(
                at + 1,
                &session,
                "wait.started",
                json!({"wait_id": format!("w{s}-{a}"), "on": agent}),
            ));
            events.push(ev(at + 2, &agent, "agent.spawned", json!({})));
            events.push(ev(
                at + 3,
                &agent,
                "agent.finished",
                json!({"status": "completed"}),
            ));
        }
        // Each guessed (once the others are paired, and the one before it
        // corrected) as g…-0, then named as its own, g…-{b + 1}.
        for call in 0..4 {
            events.push(ev(
                at + 1,
                &session,
                "spawn.requested",
                json!({"call_id": format!("g{s}-{call}"), "kind": "agent"}),
            ));
        }
        for b in 0..3 {
            let guessed = format!("{session}/b{b}");
            let when = at + 4 + 2 * b as u64;
            events.push(ev(when, &guessed, "agent.spawned", json!({})));
            events.push(ev(
                when + 1,
                &session,
                "spawn.returned",
                json!({"call_id": format!("g{s}-{}", b + 1), "child": guessed}),
            ));
        }
        // Sessions started from its shell.
        for c in 0..3 {
            let child = format!("x:c{s}-{c}");
            events.push(ev(
                at + 1,
                &session,
                "spawn.requested",
                json!({"call_id": format!("k{s}-{c}"), "kind": "session"}),
            ));
            events.push(under(
                ev(at + 2, &child, "session.started", json!({})),
                &session,
            ));
            events.push(ev(at + 3, &child, "session.ended", json!({})));
        }
        events.push(ev(at + 10, &session, "session.ended", json!({})));
    }
    events
}

/// This thread's CPU time (cycles, on Windows): unlike the time on the
/// clock, other work on the machine doesn't add to it.
fn cpu_time() -> f64 {
    #[cfg(unix)]
    {
        let mut ts = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // SAFETY: `ts` is a valid timespec to write into.
        unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut ts) };
        ts.tv_sec as f64 + ts.tv_nsec as f64 / 1e9
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Threading::GetCurrentThread;
        use windows_sys::Win32::System::WindowsProgramming::QueryThreadCycleTime;
        let mut cycles = 0u64;
        // SAFETY: the current thread's pseudo-handle, and a u64 to write into.
        unsafe { QueryThreadCycleTime(GetCurrentThread(), &mut cycles) };
        cycles as f64
    }
}

/// R22: reducing grows in step with the number of sessions, not with its
/// square: an agent's or a session's start and finish look only at its own
/// session (and the waits on it), not at every node. (Within one session,
/// they still look at its nodes and waits: see R53.)
#[test]
fn reducing_grows_in_step_with_the_history() {
    let (small, large) = (100, 1600);
    let histories = [history(small, 5), history(large, 5)];
    // The quickest of a few runs of each, taken in turn.
    let mut best = [f64::MAX; 2];
    for _ in 0..3 {
        for (i, events) in histories.iter().enumerate() {
            let events = events.clone();
            let start = cpu_time();
            let graph = reduce_at(events, 0);
            best[i] = best[i].min(cpu_time() - start);
            assert_eq!(graph.nodes.len(), [small, large][i] * 12);
        }
    }
    let ratio = best[1] / best[0];
    // 16 times the history: about 16× as long in step with it, 256× with its square.
    assert!(
        ratio < 48.0,
        "16× the history took {ratio:.1}× as long ({:.3}, then {:.3})",
        best[0],
        best[1]
    );

    // And it's still right: every child is paired, each wrong guess undone,
    // and every wait closed.
    let graph = reduce_at(history(50, 5), 0);
    for (id, node) in &graph.nodes {
        if id.contains('/') || id.starts_with("x:c") {
            assert!(node.spawned_by.is_some(), "{id} unpaired");
        }
        assert!(node.waits.iter().all(|w| !w.open), "{id} still waiting");
    }
    for s in 0..50 {
        let session = &graph.nodes[&format!("x:s{s}")];
        let child = |call: String| {
            session
                .spawns
                .iter()
                .find(|p| p.call_id == call)
                .and_then(|p| p.child.clone())
        };
        assert_eq!(child(format!("g{s}-0")), None, "the guesses undone");
        for b in 0..3 {
            let guessed = &graph.nodes[&format!("x:s{s}/b{b}")];
            let call = format!("g{s}-{}", b + 1);
            assert_eq!(guessed.spawned_by.as_deref(), Some(call.as_str()));
            assert_eq!(child(call).as_deref(), Some(guessed.id.as_str()));
        }
        for c in 0..3 {
            let child = &graph.nodes[&format!("x:c{s}-{c}")];
            assert_eq!(child.parent.as_deref(), Some(session.id.as_str()));
        }
    }
}

fn under(mut e: Envelope, parent: &str) -> Envelope {
    e.parent = Some(parent.into());
    e
}

/// R22: a session waiting on one it started stops waiting when that one
/// ends, even if it comes back later.
#[test]
fn a_wait_on_a_started_session_ends_with_it_even_if_it_resumes() {
    let graph = reduce_at(
        vec![
            ev(0, "x:p", "session.started", json!({})),
            ev(
                1,
                "x:p",
                "spawn.requested",
                json!({"call_id": "k", "kind": "session"}),
            ),
            under(ev(2, "x:c", "session.started", json!({})), "x:p"),
            ev(3, "x:c", "session.ended", json!({})),
            ev(4, "x:c", "session.started", json!({})),
        ],
        5,
    );
    let p = &graph.nodes["x:p"];
    assert!(p.waits.iter().all(|w| !w.open), "{:?}", p.waits);
    assert!(p.blocked.is_none(), "{:?}", p.blocked);
}

/// R22: a wait on a session ends with it, even if it comes back later.
#[test]
fn a_wait_ends_with_its_target_even_if_it_resumes() {
    let graph = reduce_at(
        vec![
            ev(0, "x:a", "session.started", json!({})),
            ev(0, "x:b", "session.started", json!({})),
            ev(
                1,
                "x:a",
                "wait.started",
                json!({"wait_id": "w", "on": "x:b"}),
            ),
            ev(2, "x:b", "session.ended", json!({})),
            ev(3, "x:b", "session.started", json!({})),
        ],
        4,
    );
    let a = &graph.nodes["x:a"];
    assert!(a.waits.iter().all(|w| !w.open), "{:?}", a.waits);
    assert!(a.blocked.is_none(), "{:?}", a.blocked);
}

/// R22: a request's wait left open when its child finished (which request
/// started it may still be a guess) closes when the child ends.
#[test]
fn a_spawn_wait_left_open_by_a_finish_closes_at_the_end() {
    let graph = reduce_at(
        vec![
            ev(0, "x:r", "session.started", json!({})),
            ev(
                1,
                "x:r",
                "spawn.requested",
                json!({"call_id": "c", "kind": "agent"}),
            ),
            ev(2, "x:r/a", "agent.spawned", json!({})),
            ev(3, "x:r/a", "agent.finished", json!({"status": "completed"})),
            ev(4, "x:r/a", "session.ended", json!({})),
        ],
        5,
    );
    let r = &graph.nodes["x:r"];
    assert!(r.waits.iter().all(|w| !w.open), "{:?}", r.waits);
}

/// R22: correcting a wrong guess undoes only that guess: another session's
/// request with the same call id keeps its child.
#[test]
fn correcting_a_guess_leaves_other_sessions_requests_alone() {
    let graph = reduce_at(
        vec![
            // x:t asks first, so it's first among the requests named "c".
            ev(0, "x:t", "session.started", json!({})),
            ev(
                1,
                "x:t",
                "spawn.requested",
                json!({"call_id": "c", "kind": "agent"}),
            ),
            ev(2, "x:t/a", "agent.spawned", json!({})),
            ev(3, "x:s", "session.started", json!({})),
            ev(
                4,
                "x:s",
                "spawn.requested",
                json!({"call_id": "c", "kind": "agent"}),
            ),
            ev(
                4,
                "x:s",
                "spawn.requested",
                json!({"call_id": "d", "kind": "agent"}),
            ),
            // Guessed as "c", then named as "d".
            ev(5, "x:s/a", "agent.spawned", json!({})),
            ev(
                6,
                "x:s",
                "spawn.returned",
                json!({"call_id": "d", "child": "x:s/a"}),
            ),
        ],
        7,
    );
    let child = |session: &str, call: &str| {
        graph.nodes[session]
            .spawns
            .iter()
            .find(|s| s.call_id == call)
            .and_then(|s| s.child.clone())
    };
    assert_eq!(child("x:s", "c"), None, "the wrong guess is undone");
    assert_eq!(child("x:s", "d").as_deref(), Some("x:s/a"));
    assert_eq!(child("x:t", "c").as_deref(), Some("x:t/a"), "x:t's is kept");
    assert_eq!(graph.nodes["x:t/a"].spawned_by.as_deref(), Some("c"));
}
