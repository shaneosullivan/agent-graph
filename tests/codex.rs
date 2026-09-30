//! The Codex adapter and reducer together, driven by hook payloads a real
//! Codex CLI (0.159.2) sent, in `tests/fixtures/codex/session.jsonl`: a plan,
//! a subagent it waits for, and a command that starts another agent from
//! its shell (recorded against a stand-in model: scripts/codex-e2e.sh). Then
//! a turn with an approval, a question and an interrupt, from Codex's hook
//! schema, since `codex exec` never asks for approval.

mod common;

use agent_graph::adapter::{self, Capture};
use agent_graph::event::{Payload, State, TaskStatus};
use agent_graph::reducer::{Graph, NodeKind};
use agent_graph::resume::resume;
use common::*;

const CODEX: &str = "codex:01a0f497-0990-7f13-933f-78cca4bbbb53";
const AGENT: &str =
    "codex:01a0f497-0990-7f13-933f-78cca4bbbb53/01a0f497-0a98-7791-b93f-eb49954d9567";

fn after(n: usize) -> Graph {
    let payloads = fixture("codex/session.jsonl");
    reduce(translate_as("codex", &payloads[..n], Capture::default()))
}

#[test]
fn a_session_starts_and_works() {
    let g = after(2);
    let s = &g.nodes[CODEX];
    assert_eq!(s.kind, NodeKind::Session);
    assert_eq!(s.provider, "codex");
    assert_eq!(s.state, State::Working);
    assert_eq!(s.cwd.as_deref(), Some("/home/dev/app"));
    assert_eq!(g.roots, vec![CODEX.to_string()]);
}

#[test]
fn the_plan_is_the_sessions_tasks() {
    let g = after(4);
    let statuses: Vec<_> = g.nodes[CODEX]
        .tasks
        .iter()
        .map(|t| (t.text.as_str(), t.status))
        .collect();
    assert_eq!(
        statuses,
        [
            ("Look around", TaskStatus::InProgress),
            ("Delegate the check", TaskStatus::Pending)
        ]
    );
    let g = after(17);
    assert!(
        g.nodes[CODEX]
            .tasks
            .iter()
            .all(|t| t.status == TaskStatus::Completed)
    );
}

#[test]
fn a_subagent_runs_in_the_background_under_its_session() {
    // spawn_agent returns at once, naming the child.
    let g = after(6);
    assert!(g.nodes[CODEX].blocked.is_none(), "spawning doesn't block");
    let g = after(7);
    let agent = &g.nodes[AGENT];
    assert_eq!(agent.kind, NodeKind::Agent);
    assert_eq!(agent.parent.as_deref(), Some(CODEX));
    assert_eq!(agent.agent_type.as_deref(), Some("explorer"));
    assert_eq!(agent.background, Some(true));
    assert_eq!(g.nodes[CODEX].children, vec![AGENT.to_string()]);
    // Its own events are its, not the session's.
    let g = after(11);
    assert_eq!(g.nodes[AGENT].state, State::Working);
    let g = after(12);
    assert_eq!(g.nodes[AGENT].state, State::Completed);
}

#[test]
fn waiting_for_a_subagent_blocks_until_it_returns() {
    let g = after(8);
    let blocked = g.nodes[CODEX].blocked.as_ref().expect("waiting");
    assert_eq!(blocked.on, vec![AGENT.to_string()]);
    let g = after(13);
    assert!(g.nodes[CODEX].blocked.is_none());
}

#[test]
fn starting_another_agent_from_the_shell_is_a_spawn_request() {
    let events = translate_as(
        "codex",
        &fixture("codex/session.jsonl")[13..15],
        Capture::default(),
    );
    let requested = events
        .iter()
        .find_map(|e| match Payload::parse(&e.kind, &e.data) {
            Payload::SpawnRequested(r) => Some(r),
            _ => None,
        })
        .expect("a spawn request");
    assert_eq!(requested.agent_type.as_deref(), Some("codex"));
    assert!(!requested.background);
}

#[test]
fn a_turn_ends_idle() {
    let g = after(18);
    assert_eq!(g.nodes[CODEX].state, State::Idle);
}

#[test]
fn approvals_and_questions_need_you_until_answered() {
    let g = after(20);
    let s = &g.nodes[CODEX];
    assert_eq!(s.state, State::InputRequired);
    assert_eq!(
        s.attention.as_deref(),
        Some("Needs approval: Remove the old build output")
    );
    // The command ran: approved.
    let g = after(21);
    assert_eq!(g.nodes[CODEX].state, State::Working);
    let g = after(22);
    assert_eq!(
        g.nodes[CODEX].attention.as_deref(),
        Some("Asks: Keep the cache folder?")
    );
    let g = after(23);
    assert_eq!(g.nodes[CODEX].state, State::Working);
    // Interrupted: idle, waiting for you.
    let g = after(24);
    assert_eq!(g.nodes[CODEX].state, State::Idle);
}

#[test]
fn the_session_ends() {
    let g = after(25);
    let s = &g.nodes[CODEX];
    assert_eq!(s.state, State::Completed);
    assert!(s.ended_at.is_some());
}

#[test]
fn prompts_and_agent_messages_are_kept_only_with_bodies() {
    let payloads = fixture("codex/session.jsonl");
    let text = |capture| {
        translate_as("codex", &payloads, capture)
            .iter()
            .map(|e| serde_json::to_string(e).unwrap())
            .collect::<String>()
    };
    let plain = text(Capture::default());
    assert!(!plain.contains("SUBTASK: check the files"));
    assert!(!plain.contains("Checked: all fine."));
    assert!(!plain.contains("Plan and delegate"));
    let bodies = text(Capture { bodies: true });
    assert!(bodies.contains("SUBTASK: check the files"));
    assert!(bodies.contains("Checked: all fine."));
}

#[test]
fn every_payload_translates() {
    let adapter = adapter::by_name("codex").unwrap();
    for payload in fixture("codex/session.jsonl") {
        let t = adapter
            .translate(&payload, Capture::default())
            .expect("translates");
        assert_eq!(
            t.file_key, "codex-01a0f497-0990-7f13-933f-78cca4bbbb53",
            "one file per session, its agents' too"
        );
        assert!(
            t.drafts
                .iter()
                .all(|d| !matches!(d.payload, Payload::Unknown(_)))
        );
    }
}

#[test]
fn a_session_reopens_in_codex_and_an_open_one_as_a_copy() {
    let open = resume(&after(18).nodes[CODEX]).expect("reopens");
    assert_eq!(open.program, "codex");
    assert_eq!(open.args, ["fork", "01a0f497-0990-7f13-933f-78cca4bbbb53"]);
    assert_eq!(open.cwd, "/home/dev/app");
    let ended = resume(&after(25).nodes[CODEX]).expect("reopens");
    assert_eq!(
        ended.args,
        ["resume", "01a0f497-0990-7f13-933f-78cca4bbbb53"]
    );
    // A subagent isn't a session to open.
    assert!(resume(&after(25).nodes[AGENT]).is_none());
}
