//! The Cursor adapter and reducer together, driven by the hook payloads in
//! `tests/fixtures/cursor/session.jsonl`: a todo list, two subagents started
//! in parallel (whose starts arrive out of order), one of which errors, and
//! a command that starts another agent from Cursor's shell.
//!
//! The payloads are written from Cursor's hooks docs, not captured from a
//! real Cursor: docs/cursor.md's step 1 replaces them with real ones.

mod common;

use agent_graph::adapter::{self, Capture};
use agent_graph::event::{Payload, State, TaskStatus};
use agent_graph::reducer::{Graph, NodeKind};
use common::*;

const CONV: &str = "5c1d3a7e-2b4f-4e8a-9d61-0f3c2a9b7e10";
const SESSION: &str = "cursor:5c1d3a7e-2b4f-4e8a-9d61-0f3c2a9b7e10";
const A: &str = "cursor:5c1d3a7e-2b4f-4e8a-9d61-0f3c2a9b7e10/9b2e61c4-a3f0-4d1e-8c55-7e0d4b1a2f36";
const B: &str = "cursor:5c1d3a7e-2b4f-4e8a-9d61-0f3c2a9b7e10/4f7a0d93-6e21-4b8c-b0d7-1c9e5a3f8b42";

fn payloads() -> Vec<serde_json::Value> {
    fixture("cursor/session.jsonl")
}

fn after(n: usize) -> Graph {
    reduce(translate_as("cursor", &payloads()[..n], Capture::default()))
}

#[test]
fn a_chat_is_a_session_that_works_on_your_prompt() {
    let g = after(2);
    let s = &g.nodes[SESSION];
    assert_eq!(s.kind, NodeKind::Session);
    assert_eq!(s.provider, "cursor");
    assert_eq!(s.state, State::Working);
    assert_eq!(s.cwd.as_deref(), Some("/Users/dev/app"));
    assert_eq!(g.roots, vec![SESSION.to_string()]);
}

#[test]
fn the_todo_list_is_the_sessions_tasks_and_merges_update_it() {
    let tasks = |g: &Graph| -> Vec<(String, TaskStatus)> {
        g.nodes[SESSION]
            .tasks
            .iter()
            .map(|t| (t.text.clone(), t.status))
            .collect()
    };
    assert_eq!(
        tasks(&after(3)),
        [
            ("Survey the files".to_string(), TaskStatus::InProgress),
            ("Tidy the README".to_string(), TaskStatus::Pending)
        ]
    );
    // A merge sends only what changed, by id: the text stays.
    assert_eq!(
        tasks(&after(12)),
        [
            ("Survey the files".to_string(), TaskStatus::Completed),
            ("Tidy the README".to_string(), TaskStatus::InProgress)
        ]
    );
}

#[test]
fn subagents_are_paired_with_the_task_calls_that_started_them() {
    // B starts before A, though A was asked for first: each names its call,
    // so neither is paired by guessing (which would take the oldest).
    let g = after(7);
    for (id, purpose) in [(A, "List the files in src"), (B, "Count lines in README")] {
        let agent = &g.nodes[id];
        assert_eq!(agent.kind, NodeKind::Agent);
        assert_eq!(agent.parent.as_deref(), Some(SESSION));
        assert_eq!(agent.agent_type.as_deref(), Some("explore"));
        assert_eq!(agent.purpose.as_deref(), Some(purpose), "{id}");
        assert_eq!(agent.state, State::Working);
    }
    // Both in the foreground: the session waits on them.
    let blocked = g.nodes[SESSION].blocked.as_ref().expect("waiting");
    let mut on = blocked.on.clone();
    on.sort();
    let mut both = vec![A.to_string(), B.to_string()];
    both.sort();
    assert_eq!(on, both);
}

#[test]
fn a_subagent_finishes_by_its_transcripts_name_and_the_wait_ends() {
    let g = after(8);
    assert_eq!(g.nodes[A].state, State::Completed);
    assert_eq!(g.nodes[A].summary, None, "the summary is a body");
    let g = after(11);
    assert_eq!(g.nodes[B].state, State::Failed);
    assert!(g.nodes[SESSION].blocked.is_none());
}

#[test]
fn a_subagents_summary_is_kept_only_with_body_capture() {
    let g = reduce(translate_as(
        "cursor",
        &payloads()[..8],
        Capture { bodies: true },
    ));
    assert_eq!(g.nodes[A].summary.as_deref(), Some("src has 12 files."));
}

#[test]
fn a_subagents_prompt_isnt_kept_without_body_capture() {
    // Its start, alone, with no description: `task` is its prompt.
    let mut start = payloads()[6].clone();
    start["description"] = serde_json::Value::Null;
    let g = reduce(translate_as(
        "cursor",
        &[payloads()[0].clone(), start],
        Capture::default(),
    ));
    assert_eq!(g.nodes[A].purpose, None);
}

#[test]
fn a_command_that_starts_an_agent_is_a_spawn_request() {
    let events = translate_as("cursor", &payloads()[12..13], Capture::default());
    let Payload::SpawnRequested(r) = events[0].payload() else {
        panic!("not a spawn request: {:?}", events[0]);
    };
    assert_eq!(r.call_id, "toolu_shell1");
    assert_eq!(r.agent_type.as_deref(), Some("claude"));
}

#[test]
fn the_turn_ending_is_idle_and_closing_the_chat_ends_it() {
    let g = after(15);
    assert_eq!(g.nodes[SESSION].state, State::Idle);
    let g = after(16);
    assert_eq!(g.nodes[SESSION].state, State::Completed);
}

#[test]
fn a_question_or_plan_needs_you_once_cursor_sends_them() {
    let ask = serde_json::json!({
        "conversation_id": CONV, "hook_event_name": "preToolUse",
        "tool_name": "AskQuestion", "tool_use_id": "toolu_ask",
        "tool_input": {"questions": [{"prompt": "Which README?"}]},
    });
    let plan = serde_json::json!({
        "conversation_id": CONV, "hook_event_name": "preToolUse",
        "tool_name": "CreatePlan", "tool_use_id": "toolu_plan", "tool_input": {},
    });
    let g = reduce(translate_as(
        "cursor",
        &[payloads()[0].clone(), ask],
        Capture::default(),
    ));
    assert_eq!(g.nodes[SESSION].state, State::InputRequired);
    assert_eq!(
        g.nodes[SESSION].attention.as_deref(),
        Some("Asks: Which README?")
    );
    let g = reduce(translate_as(
        "cursor",
        &[payloads()[0].clone(), plan],
        Capture::default(),
    ));
    assert_eq!(
        g.nodes[SESSION].attention.as_deref(),
        Some("Plan ready for your review")
    );
}

#[test]
fn a_session_that_errors_fails() {
    let mut end = payloads()[15].clone();
    end["reason"] = "error".into();
    end["error_message"] = "Model unavailable".into();
    let g = reduce(translate_as(
        "cursor",
        &[payloads()[0].clone(), end],
        Capture::default(),
    ));
    assert_eq!(g.nodes[SESSION].state, State::Failed);
}

#[test]
fn cursor_hooks_answer_with_json() {
    let cursor = adapter::by_name("cursor").unwrap();
    assert!(cursor.replies_with_json());
    assert!(!adapter::by_name("codex").unwrap().replies_with_json());
    assert_eq!(agent_graph::emit::json_reply(&[]), "{}");
    let reply = agent_graph::emit::json_reply(&[("AGENT_GRAPH_PARENT".into(), SESSION.into())]);
    let reply: serde_json::Value = serde_json::from_str(&reply).unwrap();
    assert_eq!(reply["env"]["AGENT_GRAPH_PARENT"], SESSION);
}
