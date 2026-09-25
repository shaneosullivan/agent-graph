//! Sessions that start other sessions: linking them (by environment, by
//! `agent-graph run`, or by process), and a session waiting on one it
//! started from its shell. See docs/design.md §5.2 and §6.

mod common;

use std::time::Duration;

use agent_graph::adapter::{self, Capture};
use agent_graph::emit::stamp;
use agent_graph::event::{Envelope, Payload, SessionStarted, Source, State};
use agent_graph::reducer::Graph;
use common::{reduce, t0};
use serde_json::{Value, json};

const A: &str = "claude-code:aaaa";
const B: &str = "claude-code:bbbb";

/// Events from Claude Code hook payloads, one second apart from `at` seconds.
fn claude(at: u64, payloads: &[Value]) -> Vec<Envelope> {
    let adapter = adapter::by_name("claude-code").unwrap();
    let source = source("claude-code");
    payloads
        .iter()
        .enumerate()
        .flat_map(|(i, p)| {
            let t = adapter
                .translate(p, Capture::default())
                .expect("translates");
            stamp(t.drafts, &source, t0() + Duration::from_secs(at + i as u64))
        })
        .collect()
}

fn source(provider: &str) -> Source {
    Source {
        provider: provider.into(),
        provider_version: None,
        adapter: None,
    }
}

fn hook(session: &str, event: &str, extra: Value) -> Value {
    let mut payload = json!({ "session_id": session, "hook_event_name": event, "cwd": "/w/app" });
    payload
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    payload
}

fn bash(session: &str, pre: bool, id: &str, command: &str) -> Value {
    hook(
        session,
        if pre { "PreToolUse" } else { "PostToolUse" },
        json!({
            "tool_name": "Bash",
            "tool_use_id": id,
            "tool_input": { "command": command, "description": format!("Run {command}") },
        }),
    )
}

/// A session starting as `emit` records it: with the parent it inherited,
/// or with its processes.
fn started(
    at: u64,
    node: &str,
    parent: Option<&str>,
    process: Option<&str>,
    ancestors: &[&str],
) -> Envelope {
    let provider = node.split(':').next().unwrap();
    let mut e = stamp(
        vec![adapter::Draft::new(
            node,
            Payload::SessionStarted(SessionStarted {
                cwd: Some("/w/app".into()),
                link_method: parent.map(|p| agent_graph::link::method_for(p).to_string()),
                process: process.map(String::from),
                ancestors: ancestors.iter().map(|s| s.to_string()).collect(),
                title: (provider == "run").then(|| "workers.sh".to_string()),
                ..Default::default()
            }),
        )],
        &source(provider),
        t0() + Duration::from_secs(at),
    )
    .remove(0);
    e.parent = parent.map(String::from);
    e
}

fn event(at: u64, node: &str, payload: Payload) -> Envelope {
    let provider = node.split(':').next().unwrap();
    stamp(
        vec![adapter::Draft::new(node, payload)],
        &source(provider),
        t0() + Duration::from_secs(at),
    )
    .remove(0)
}

fn graph(events: Vec<Vec<Envelope>>) -> Graph {
    reduce(events.into_iter().flatten().collect())
}

#[test]
fn only_bash_commands_that_start_an_agent_are_recorded() {
    for command in [
        "ls -la",
        "cargo test",
        "echo 'claude -p hi'",
        "claude --version",
    ] {
        assert!(
            claude(
                0,
                &[
                    bash("aaaa", true, "t", command),
                    bash("aaaa", false, "t", command)
                ]
            )
            .is_empty(),
            "{command}"
        );
    }
    let events = claude(0, &[bash("aaaa", true, "t", "claude -p 'fix it'")]);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, "spawn.requested");
    assert_eq!(events[0].data["kind"], "session");
    assert_eq!(events[0].data["agent_type"], "claude");
    assert_eq!(events[0].data["background"], false);
    assert_eq!(events[0].data["purpose"], "Run claude -p 'fix it'");
}

#[test]
fn a_session_waits_on_one_it_starts_from_its_shell() {
    let parent = claude(
        0,
        &[
            hook("aaaa", "SessionStart", json!({})),
            hook("aaaa", "UserPromptSubmit", json!({})),
            bash("aaaa", true, "toolu_1", "claude -p 'write the tests'"),
        ],
    );
    let child_start = vec![started(10, B, Some(A), None, &[])];
    let child_work = claude(11, &[hook("bbbb", "UserPromptSubmit", json!({}))]);

    // Asked for, not started yet.
    let g = graph(vec![parent.clone()]);
    assert_eq!(g.nodes[A].blocked.as_ref().unwrap().starting, 1);

    // Started: it's under the session, which is waiting on it.
    let g = graph(vec![
        parent.clone(),
        child_start.clone(),
        child_work.clone(),
    ]);
    let child = &g.nodes[B];
    assert_eq!(child.parent.as_deref(), Some(A));
    assert_eq!(child.link.as_deref(), Some("env"));
    assert_eq!(child.spawned_by.as_deref(), Some("toolu_1"));
    assert_eq!(
        child.purpose.as_deref(),
        Some("Run claude -p 'write the tests'")
    );
    assert_eq!(g.nodes[A].blocked.as_ref().unwrap().on, [B]);
    assert_eq!(g.roots, [A], "not a root of its own any more");

    // It ends, and the shell command returns: nothing's waiting.
    let done = vec![
        claude(20, &[hook("bbbb", "SessionEnd", json!({}))]),
        claude(
            21,
            &[bash(
                "aaaa",
                false,
                "toolu_1",
                "claude -p 'write the tests'",
            )],
        ),
    ];
    let g = graph([vec![parent, child_start, child_work], done].concat());
    assert!(g.nodes[A].blocked.is_none());
    assert_eq!(g.nodes[B].state, State::Completed);
}

#[test]
fn a_background_launch_pairs_with_the_session_it_starts() {
    let events = vec![
        claude(
            0,
            &[
                hook("aaaa", "SessionStart", json!({})),
                bash(
                    "aaaa",
                    true,
                    "toolu_bg",
                    "codex exec 'long job' > log 2>&1 &",
                ),
                bash(
                    "aaaa",
                    false,
                    "toolu_bg",
                    "codex exec 'long job' > log 2>&1 &",
                ),
            ],
        ),
        // The shell returned at once; the child's first hook comes after.
        vec![started(5, "codex:cccc", Some(A), None, &[])],
    ];
    let g = graph(events);
    let child = &g.nodes["codex:cccc"];
    assert_eq!(child.parent.as_deref(), Some(A));
    assert_eq!(child.spawned_by.as_deref(), Some("toolu_bg"));
    assert_eq!(child.background, Some(true));
    assert!(g.nodes[A].blocked.is_none(), "background: not waited on");
}

#[test]
fn parallel_launches_pair_by_program() {
    let events = vec![
        claude(
            0,
            &[
                hook("aaaa", "SessionStart", json!({})),
                bash("aaaa", true, "toolu_codex", "codex exec 'review'"),
                bash("aaaa", true, "toolu_claude", "claude -p 'docs'"),
            ],
        ),
        vec![started(5, B, Some(A), None, &[])],
    ];
    let g = graph(events);
    assert_eq!(g.nodes[B].spawned_by.as_deref(), Some("toolu_claude"));
    let blocked = g.nodes[A].blocked.as_ref().unwrap();
    assert_eq!(blocked.on, [B]);
    assert_eq!(blocked.starting, 1, "codex hasn't started");
}

#[test]
fn a_session_is_linked_by_its_processes_when_the_environment_is_missing() {
    let events = vec![
        vec![started(0, A, None, Some("100@7"), &["50@1"])],
        // Its agent (300) runs under a shell (200) that A's agent (100) started.
        vec![started(
            5,
            B,
            None,
            Some("300@9"),
            &["200@8", "100@7", "50@1"],
        )],
        // A different process that reused pid 100 isn't A's agent.
        vec![started(
            6,
            "claude-code:cccc",
            None,
            Some("400@12"),
            &["100@11"],
        )],
    ];
    let g = graph(events);
    assert_eq!(g.nodes[B].parent.as_deref(), Some(A));
    assert_eq!(g.nodes[B].link.as_deref(), Some("process"));
    assert_eq!(g.nodes["claude-code:cccc"].parent, None);
    assert!(g.roots.contains(&"claude-code:cccc".to_string()));
}

#[test]
fn a_session_started_by_clear_in_the_same_process_is_not_its_own_child() {
    // `/clear` starts a new session in the same agent process.
    let events = vec![
        vec![started(0, A, None, Some("100@7"), &["50@1"])],
        vec![started(5, B, None, Some("100@7"), &["50@1"])],
    ];
    let g = graph(events);
    assert_eq!(g.nodes[B].parent, None);
    assert_eq!(g.nodes[A].parent, None);
}

#[test]
fn agent_graph_run_groups_what_it_starts_and_keeps_its_failure() {
    const RUN: &str = "run:01K0";
    let events = vec![
        vec![started(0, RUN, None, Some("10@1"), &[])],
        vec![event(
            0,
            RUN,
            Payload::parse("status", &json!({"state": "working"})),
        )],
        vec![started(1, B, Some(RUN), None, &[])],
        vec![started(2, "claude-code:cccc", Some(RUN), None, &[])],
        vec![event(
            9,
            RUN,
            Payload::parse(
                "status",
                &json!({"state": "failed", "summary": "Exited with code 3"}),
            ),
        )],
        vec![event(
            9,
            RUN,
            Payload::parse("session.ended", &json!({"reason": "exit 3"})),
        )],
    ];
    let g = graph(events);
    let run = &g.nodes[RUN];
    assert_eq!(run.children, [B, "claude-code:cccc"]);
    assert_eq!(g.nodes[B].link.as_deref(), Some("run"));
    assert_eq!(run.state, State::Failed, "session.ended keeps the failure");
    assert_eq!(run.summary.as_deref(), Some("Exited with code 3"));
    assert_eq!(g.roots, [RUN]);
}

#[test]
fn ending_a_session_cancels_its_agents_but_not_sessions_it_started() {
    let events = vec![
        claude(
            0,
            &[
                hook("aaaa", "SessionStart", json!({})),
                hook(
                    "aaaa",
                    "SubagentStart",
                    json!({"agent_id": "ag1", "agent_type": "Explore"}),
                ),
                bash("aaaa", true, "toolu_bg", "claude -p 'keep going' &"),
            ],
        ),
        vec![started(5, B, Some(A), None, &[])],
        claude(6, &[hook("bbbb", "UserPromptSubmit", json!({}))]),
        claude(9, &[hook("aaaa", "SessionEnd", json!({}))]),
    ];
    let g = graph(events);
    assert_eq!(g.nodes[&format!("{A}/ag1")].state, State::Canceled);
    assert_eq!(g.nodes[B].state, State::Working, "a process of its own");
}
