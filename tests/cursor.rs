//! The Cursor adapter and reducer together, driven by hook payloads a real
//! Cursor sent (the app, 3.23.12, and its CLI, 2026.10.01), in
//! `tests/fixtures/cursor/session.jsonl`, with paths, email and outputs
//! scrubbed. In order:
//!
//! - a chat in the app (0–18, 28–30): two subagents started in parallel,
//!   one of which finishes; then three more messages sent while it's still
//!   at work, each stopping the turn before (`stop` with `aborted`), and the
//!   last running commands, one of which starts Claude Code;
//! - a chat in the CLI, `agent` (19–23): two turns;
//! - `agent -p` (24–27): a command, then the end, with no prompt or `stop`.
//!
//! The subagent's own hooks (6, 7) carry its own conversation and the
//! `Task` call that started it, and nothing naming the chat.

mod common;

use agent_graph::adapter::{self, Capture};
use agent_graph::event::{Payload, State};
use agent_graph::reducer::{Graph, NodeKind};
use common::*;
use serde_json::Value;

const APP: &str = "cursor:516781fb-4582-42b3-b4f4-8a294572e5f6";
const CLI: &str = "cursor:385fd5a6-3d88-4cc1-8ebd-a6883b0e4fbe";
const PRINT: &str = "cursor:e7e93694-25de-4dcf-a0c8-4d52e9dc1100";

fn payloads() -> Vec<Value> {
    fixture("cursor/session.jsonl")
}

fn after(n: usize) -> Graph {
    reduce(translate_as("cursor", &payloads()[..n], Capture::default()))
}

/// The node of the subagent payload `i` starts: its `subagent_id`'s first
/// line (Cursor's ids hold a newline).
fn subagent(i: usize) -> String {
    let id = payloads()[i]["subagent_id"].as_str().unwrap().to_string();
    assert!(id.contains('\n'), "the fixture keeps Cursor's newline");
    format!("{APP}/{}", id.lines().next().unwrap())
}

#[test]
fn a_chat_is_a_session_that_works_on_your_prompt() {
    let g = after(2);
    let s = &g.nodes[APP];
    assert_eq!(s.kind, NodeKind::Session);
    assert_eq!(s.provider, "cursor");
    assert_eq!(s.state, State::Working);
    assert_eq!(s.cwd.as_deref(), Some("/Users/dev/app"));
    assert_eq!(g.roots, vec![APP.to_string()]);
}

#[test]
fn subagents_are_paired_with_the_task_calls_that_started_them() {
    // They start in the opposite order to their calls: each is paired by
    // its call, not guessed (which would take the oldest).
    let g = after(6);
    let (list, count) = (subagent(4), subagent(5));
    for (id, purpose) in [(&list, "List files in src"), (&count, "Count README lines")] {
        let agent = &g.nodes[id];
        assert_eq!(agent.kind, NodeKind::Agent);
        assert_eq!(agent.parent.as_deref(), Some(APP));
        assert_eq!(agent.agent_type.as_deref(), Some("explore"));
        assert_eq!(agent.purpose.as_deref(), Some(purpose), "{id}");
        assert_eq!(agent.state, State::Working);
        assert!(!id.contains('\n'));
    }
    // Both in the foreground: the chat waits on them.
    let blocked = g.nodes[APP].blocked.as_ref().expect("waiting");
    let mut on = blocked.on.clone();
    on.sort();
    let mut both = vec![list, count];
    both.sort();
    assert_eq!(on, both);
}

#[test]
fn a_subagents_own_hooks_are_left_out() {
    let events = translate_as("cursor", &payloads()[6..8], Capture::default());
    assert!(events.is_empty(), "{events:?}");
}

#[test]
fn a_subagent_finishes_and_one_stopped_with_its_turn_is_canceled() {
    let g = after(9);
    assert_eq!(g.nodes[&subagent(5)].state, State::Completed);
    assert_eq!(g.nodes[&subagent(5)].summary, None, "the summary is a body");
    assert_eq!(g.nodes[&subagent(4)].state, State::Working);
    // A message sent mid-turn stops it: Cursor says nothing more of the
    // subagent still at work, nor of either Task call.
    let g = after(10);
    assert_eq!(g.nodes[APP].state, State::Idle);
    assert_eq!(g.nodes[&subagent(4)].state, State::Canceled);
    assert!(g.nodes[APP].blocked.is_none());
    let g = after(11);
    assert_eq!(g.nodes[APP].state, State::Working);
}

#[test]
fn a_command_that_starts_an_agent_is_a_spawn_request() {
    let events = translate_as("cursor", &payloads()[28..29], Capture::default());
    let Payload::SpawnRequested(r) = events[0].payload() else {
        panic!("not a spawn request: {:?}", events[0]);
    };
    assert_eq!(r.agent_type.as_deref(), Some("claude"));
    // Other commands aren't recorded.
    let events = translate_as("cursor", &payloads()[15..19], Capture::default());
    assert!(events.is_empty(), "{events:?}");
}

#[test]
fn the_chats_turn_ends_idle() {
    let g = after(31);
    assert_eq!(g.nodes[APP].state, State::Idle);
}

#[test]
fn a_cli_chat_works_and_idles_turn_by_turn() {
    let g = after(21);
    assert_eq!(g.nodes[CLI].state, State::Working);
    let g = after(22);
    assert_eq!(g.nodes[CLI].state, State::Idle);
    let g = after(24);
    assert_eq!(g.nodes[CLI].state, State::Idle);
}

#[test]
fn a_print_run_ends_when_it_ends() {
    let g = after(28);
    assert_eq!(g.nodes[PRINT].state, State::Completed);
    assert_eq!(g.roots.len(), 3);
}

#[test]
fn a_question_or_plan_needs_you_once_cursor_sends_them() {
    // Cursor doesn't yet run hooks for these (a bug its staff have
    // confirmed): this is the shape its docs give.
    let conv = "516781fb-4582-42b3-b4f4-8a294572e5f6";
    let ask = serde_json::json!({
        "conversation_id": conv, "hook_event_name": "preToolUse",
        "tool_name": "AskQuestion", "tool_use_id": "toolu_ask",
        "tool_input": {"questions": [{"prompt": "Which README?"}]},
    });
    let plan = serde_json::json!({
        "conversation_id": conv, "hook_event_name": "preToolUse",
        "tool_name": "CreatePlan", "tool_use_id": "toolu_plan", "tool_input": {},
    });
    let start = payloads()[0].clone();
    let g = reduce(translate_as(
        "cursor",
        &[start.clone(), ask],
        Capture::default(),
    ));
    assert_eq!(g.nodes[APP].state, State::InputRequired);
    assert_eq!(
        g.nodes[APP].attention.as_deref(),
        Some("Asks: Which README?")
    );
    let g = reduce(translate_as("cursor", &[start, plan], Capture::default()));
    assert_eq!(
        g.nodes[APP].attention.as_deref(),
        Some("Plan ready for your review")
    );
}

#[test]
fn a_session_that_errors_fails() {
    let mut end = payloads()[27].clone();
    end["reason"] = "error".into();
    end["error_message"] = "Model unavailable".into();
    let start = payloads()[24].clone();
    let g = reduce(translate_as("cursor", &[start, end], Capture::default()));
    assert_eq!(g.nodes[PRINT].state, State::Failed);
}

#[test]
fn cursor_hooks_answer_with_json() {
    assert!(adapter::by_name("cursor").unwrap().replies_with_json());
    assert!(!adapter::by_name("codex").unwrap().replies_with_json());
    assert!(!adapter::by_name("claude-code").unwrap().replies_with_json());
}

#[test]
fn claude_codes_hooks_ignore_the_payloads_cursor_runs_them_with() {
    // Cursor loads Claude Code's hooks as well as its own, and runs them
    // with its payloads: they'd make a Claude Code session of each chat.
    for payload in payloads() {
        let t = adapter::by_name("claude-code")
            .unwrap()
            .translate(&payload, Capture::default())
            .unwrap();
        assert!(t.drafts.is_empty(), "{payload}");
    }
}
