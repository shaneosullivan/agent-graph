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

/// A session `child` (from provider `provider`, with `data`) started under `x:p`.
fn shell_child(secs: u64, child: &str, provider: &str, data: Value) -> Envelope {
    let mut e = under(ev(secs, child, "session.started", data), "x:p");
    e.source = Some(agent_graph::event::Source {
        provider: provider.into(),
        provider_version: None,
        adapter: None,
    });
    e
}

fn session_request(secs: u64, call: &str, program: Option<&str>, background: bool) -> Envelope {
    let mut data = json!({"call_id": call, "kind": "session", "background": background});
    if let Some(p) = program {
        data["agent_type"] = json!(p);
    }
    ev(secs, "x:p", "spawn.requested", data)
}

/// Which request the child session `child` was paired with.
fn paired_with(events: Vec<Envelope>, child: &str) -> Option<String> {
    reduce_at(events, 100_000).nodes[child].spawned_by.clone()
}

/// R24: a session started from a shell isn't paired with a request for
/// another program it surely isn't running (it would take that request's
/// purpose and type, for good), but anything that might be it still is.
#[test]
fn a_shell_session_isnt_paired_with_another_programs_request() {
    let start = || ev(0, "x:p", "session.started", json!({}));
    let claude = || shell_child(5, "claude-code:c", "claude-code", json!({}));

    // codex is another agent CLI, which Claude Code sessions don't run.
    assert_eq!(
        paired_with(
            vec![
                start(),
                session_request(1, "k", Some("codex"), false),
                claude()
            ],
            "claude-code:c"
        ),
        None
    );
    // Its own program's; one whose program isn't known; a command added
    // with AGENT_GRAPH_AGENT_COMMANDS (a wrapper, say).
    for program in [Some("claude"), None, Some("ccr")] {
        assert_eq!(
            paired_with(
                vec![start(), session_request(1, "k", program, false), claude()],
                "claude-code:c"
            )
            .as_deref(),
            Some("k"),
            "{program:?}"
        );
    }
    // `agent-graph run --name build -- claude …`: named for the run, so it might be anything.
    let run = shell_child(5, "run:r", "run", json!({"title": "build"}));
    assert_eq!(
        paired_with(
            vec![start(), session_request(1, "k", Some("claude"), false), run],
            "run:r"
        )
        .as_deref(),
        Some("k")
    );

    // Its own program's first, then the oldest: before an older one whose
    // program isn't known, and before a newer one of its own.
    assert_eq!(
        paired_with(
            vec![
                start(),
                session_request(1, "unknown", None, false),
                session_request(2, "own", Some("claude"), false),
                session_request(3, "newer", Some("claude"), false),
                claude(),
            ],
            "claude-code:c"
        )
        .as_deref(),
        Some("own")
    );
    // None for its own program: the oldest that might be it.
    assert_eq!(
        paired_with(
            vec![
                start(),
                session_request(1, "older", None, false),
                session_request(2, "newer", Some("ccr"), false),
                claude(),
            ],
            "claude-code:c"
        )
        .as_deref(),
        Some("older")
    );
    // An `agent-graph run -- claude …` runs `claude`, too.
    let run = shell_child(5, "run:r", "run", json!({"title": "claude"}));
    assert_eq!(
        paired_with(
            vec![
                start(),
                session_request(1, "unknown", None, false),
                session_request(2, "own", Some("claude"), false),
                run,
            ],
            "run:r"
        )
        .as_deref(),
        Some("own")
    );
}

/// R24: a background request returns at once, so a session started long
/// after it (ten minutes, allowing for what the command did first) isn't the
/// one it launched; a foreground request waits for its child, however long.
#[test]
fn only_a_recent_background_request_is_paired_with_a_shell_session() {
    let paired = |gap: u64, background: bool| {
        paired_with(
            vec![
                ev(0, "x:p", "session.started", json!({})),
                session_request(1, "k", Some("claude"), background),
                shell_child(1 + gap, "claude-code:c", "claude-code", json!({})),
            ],
            "claude-code:c",
        )
    };
    assert_eq!(paired(5, true).as_deref(), Some("k"), "just after");
    assert_eq!(paired(600, true).as_deref(), Some("k"), "ten minutes after");
    assert_eq!(paired(601, true), None, "longer");
    assert_eq!(
        paired(3600, false).as_deref(),
        Some("k"),
        "in the foreground"
    );

    // A background request too old to have launched it just now (it may
    // never launch anything) comes after a fresh one...
    let pick = |background_at: u64, foreground_at: u64| {
        paired_with(
            vec![
                ev(0, "x:p", "session.started", json!({})),
                session_request(background_at, "bg", Some("claude"), true),
                session_request(foreground_at, "fg", Some("claude"), false),
                shell_child(200, "claude-code:c", "claude-code", json!({})),
            ],
            "claude-code:c",
        )
    };
    assert_eq!(pick(20, 199).as_deref(), Some("fg"), "three minutes old");
    assert_eq!(pick(139, 199).as_deref(), Some("fg"), "61 s old");
    // ...but a fresh one is taken oldest first (`claude -p a &`, then `claude -p b`).
    assert_eq!(pick(198, 199).as_deref(), Some("bg"));
    assert_eq!(pick(140, 199).as_deref(), Some("bg"), "60 s old");
    // A foreground request is never too old: it's still waiting for its
    // child (`make build && claude -p …`).
    assert_eq!(pick(199, 20).as_deref(), Some("fg"));
}

/// Events in the same millisecond, in this order: their ids are made to
/// sort as given (hooks in separate processes get random ones, which can
/// sort either way).
fn at_once(mut events: Vec<Envelope>) -> Vec<Envelope> {
    for (i, e) in events.iter_mut().enumerate() {
        e.id = format!("{}{i:016}", &e.id[..10]);
    }
    events
}

/// R28: a headless session's Stop and SessionEnd can land in the same
/// millisecond, in either order. A status sorted just after
/// `session.ended` (within `LATE_STATUS`) is a late one, and doesn't bring
/// the session (or its agents) back to life; a later one, or a new
/// `session.started`, does.
#[test]
fn a_late_status_doesnt_revive_an_ended_session() {
    let base = || {
        vec![
            ev(1, "x:s", "session.started", json!({})),
            ev(2, "x:s", "status", json!({"state": "working"})),
            ev(2, "x:s/a", "agent.spawned", json!({})),
        ]
    };
    let ended_then = |late: Vec<Envelope>| {
        let mut events = base();
        let mut at_end = vec![ev(3, "x:s", "session.ended", json!({}))];
        at_end.extend(late);
        events.extend(at_once(at_end));
        events
    };
    let status = |node: &str, state: &str| {
        ev(
            3,
            node,
            "status",
            json!({"state": state, "summary": "late"}),
        )
    };
    for state in ["idle", "working", "input_required"] {
        let g = reduce_at(ended_then(vec![status("x:s", state)]), 10);
        let s = &g.nodes["x:s"];
        assert_eq!(s.state, State::Completed, "{state}");
        assert!(s.ended_at.is_some(), "{state}");
        assert_eq!((s.attention.as_deref(), s.summary.as_deref()), (None, None));
        assert!(g.late.iter().any(|id| id.ends_with('1')), "{state}");
        // Nor its agents, which ended with it.
        let g = reduce_at(ended_then(vec![status("x:s/a", state)]), 10);
        assert_eq!(g.nodes["x:s/a"].state, State::Canceled, "{state}");
    }
    // A status that says how it ended still counts.
    let g = reduce_at(ended_then(vec![status("x:s", "failed")]), 10);
    assert_eq!(g.nodes["x:s"].state, State::Failed);
    assert!(g.late.is_empty());
    // A failure then the end (as `agent-graph run` reports them) stays so.
    let mut events = base();
    events.extend(at_once(vec![
        ev(3, "x:s", "status", json!({"state": "failed"})),
        ev(3, "x:s", "session.ended", json!({})),
        ev(3, "x:s", "status", json!({"state": "idle"})),
    ]));
    assert_eq!(reduce_at(events, 10).nodes["x:s"].state, State::Failed);

    // Later activity means it's running after all (another process on the
    // same conversation, say): that counts, as ever.
    for (at, state) in [
        (4, State::Completed),
        (5, State::Working),
        (6, State::Working),
    ] {
        let mut events = ended_then(vec![]);
        events.push(ev(at, "x:s", "status", json!({"state": "working"})));
        assert_eq!(reduce_at(events, 10).nodes["x:s"].state, state, "{at}");
    }
    // So does a new start, whatever its source, even in the same moment.
    for source in ["startup", "resume", "clear", "compact", "run"] {
        let events = ended_then(vec![
            ev(3, "x:s", "session.started", json!({"source": source})),
            ev(3, "x:s", "status", json!({"state": "working"})),
        ]);
        let g = reduce_at(events, 10);
        assert_eq!(g.nodes["x:s"].state, State::Working, "{source}");
        assert_eq!(g.nodes["x:s"].ended_at, None, "{source}");
    }
    // An agent first seen in a late status ended with its session too.
    let g = reduce_at(ended_then(vec![status("x:s/b", "idle")]), 10);
    let b = &g.nodes["x:s/b"];
    assert_eq!(b.state, State::Canceled);
    assert_eq!(b.ended_at, g.nodes["x:s"].ended_at);

    // An end that comes with a start (a resumed run's start, and the run
    // before it's end, landing together) doesn't make the new run's
    // statuses late.
    let mut events = base();
    events.push(ev(60, "x:s", "status", json!({"state": "idle"})));
    events.extend(at_once(vec![
        ev(61, "x:s", "session.started", json!({"source": "resume"})),
        ev(61, "x:s", "session.ended", json!({})),
    ]));
    events.push(ev(62, "x:s", "status", json!({"state": "working"})));
    let g = reduce_at(events, 90);
    assert_eq!(g.nodes["x:s"].state, State::Working);
    assert!(g.late.is_empty());
    // There, only a Stop is late: a run that ends within `LATE_STATUS` of
    // starting (a quick `claude -p`) still ends, whichever order its last
    // hooks land in, and a status that says it's busy counts.
    let short = |last: &str| {
        let mut events = vec![
            ev(0, "x:s", "session.started", json!({})),
            ev(0, "x:s", "status", json!({"state": "working"})),
        ];
        events.extend(at_once(vec![
            ev(1, "x:s", "session.ended", json!({})),
            ev(1, "x:s", "status", json!({"state": last})),
        ]));
        reduce_at(events, 90).nodes["x:s"].state
    };
    assert_eq!(short("idle"), State::Completed);
    assert_eq!(short("input_required"), State::InputRequired);
    // "With a start": less than `LATE_STATUS` after it.
    for (started, state) in [(60, State::Working), (59, State::Completed)] {
        let events = vec![
            ev(started, "x:s", "session.started", json!({})),
            ev(61, "x:s", "session.ended", json!({})),
            ev(62, "x:s", "status", json!({"state": "working"})),
        ];
        assert_eq!(reduce_at(events, 90).nodes["x:s"].state, state, "{started}");
    }

    // Only a session's end makes a status late: not an agent's finish, nor
    // a terminal status.
    let mut events = base();
    events.extend(at_once(vec![
        ev(3, "x:s/a", "agent.finished", json!({"status": "completed"})),
        ev(3, "x:s/a", "status", json!({"state": "working"})),
        ev(3, "x:s", "status", json!({"state": "failed"})),
        ev(3, "x:s", "status", json!({"state": "working"})),
    ]));
    let g = reduce_at(events, 10);
    assert_eq!(g.nodes["x:s/a"].state, State::Working);
    assert_eq!(g.nodes["x:s"].state, State::Working);
}
