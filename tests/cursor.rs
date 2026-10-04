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
use agent_graph::event::{Payload, State, TaskStatus};
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

/// A CLI chat reopens in the CLI, but not mid-turn (the CLI has no copy of
/// a chat to open instead).
#[test]
fn a_cli_chat_reopens_in_the_cli_between_turns() {
    use agent_graph::resume::cursor_cli;
    assert!(cursor_cli(&after(21).nodes[CLI]).is_none(), "mid-turn");
    let r = cursor_cli(&after(22).nodes[CLI]).expect("between turns");
    assert_eq!(r.program, "agent");
    let id = CLI.strip_prefix("cursor:").unwrap();
    assert_eq!(r.args, ["--resume", id]);
    assert!(!r.cwd.is_empty());
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

// ---------------------------------------------------------------------------
// Approvals and questions, from `tests/fixtures/cursor/approvals.jsonl`: Cursor
// set to ask before running commands (its Allowlist run mode, with nothing
// on the list). In the app (0–11), two commands, each approved after about
// 20 s; in the CLI (12–20), one, the same. Hooks come before Cursor asks
// (`beforeShellExecution`), and after the command has run.

fn approvals() -> Vec<Value> {
    fixture("cursor/approvals.jsonl")
}

/// The graph after the first `n` approvals payloads (one a second), seen
/// `waited` seconds after the last of them.
fn approvals_after(n: usize, waited: u64, capture: Capture) -> Graph {
    let events = translate_as("cursor", &approvals()[..n], capture);
    agent_graph::reducer::reduce(
        events,
        &agent_graph::reducer::Options {
            now: t0() + std::time::Duration::from_secs(n as u64 - 1 + waited),
            stale_after: std::time::Duration::from_secs(30 * 60),
        },
    )
}

fn app_node(g: &Graph) -> &agent_graph::reducer::Node {
    let id = format!(
        "cursor:{}",
        approvals()[0]["conversation_id"].as_str().unwrap()
    );
    &g.nodes[&id]
}

#[test]
fn a_command_outside_the_sandbox_may_be_waiting_for_you_after_a_while() {
    // Just started: running, as far as anyone can tell.
    let g = approvals_after(4, 3, Capture::default());
    assert_eq!(app_node(&g).state, State::Working);
    // Twenty seconds on, with nothing since: probably asking, and it says
    // it's a guess.
    let g = approvals_after(4, 20, Capture::default());
    assert_eq!(app_node(&g).state, State::InputRequired);
    assert_eq!(
        app_node(&g).attention.as_deref(),
        Some(agent_graph::reducer::MAY_ASK)
    );
    // It ran: working again.
    let g = approvals_after(5, 20, Capture::default());
    assert_eq!(app_node(&g).state, State::Working);
}

#[test]
fn a_page_knows_when_to_look_again_for_a_command_that_may_be_asking() {
    // The command started at payload 3 (3 s in): shown as asking 10 s on,
    // with nothing new, so a page showing the graph sooner looks again then.
    let g = approvals_after(4, 3, Capture::default());
    assert_eq!(
        g.recheck_at,
        Some(t0() + std::time::Duration::from_secs(3) + agent_graph::reducer::MAY_ASK_AFTER)
    );
    // Once it's shown as asking, or the command has run, nothing's due.
    assert_eq!(approvals_after(4, 20, Capture::default()).recheck_at, None);
    assert_eq!(approvals_after(5, 0, Capture::default()).recheck_at, None);
}

#[test]
fn a_command_in_the_sandbox_never_asks() {
    let mut before = approvals()[3].clone();
    before["sandbox"] = true.into();
    let mut after = approvals()[4].clone();
    after["sandbox"] = true.into();
    let events = translate_as("cursor", &[before, after], Capture::default());
    assert!(events.is_empty(), "{events:?}");
}

#[test]
fn a_command_you_didnt_allow_isnt_waiting_on_you() {
    let mut refused = approvals()[5].clone();
    refused["hook_event_name"] = "postToolUseFailure".into();
    refused["failure_type"] = "permission_denied".into();
    let mut payloads = approvals()[..4].to_vec();
    payloads.push(refused);
    let g = reduce(translate_as("cursor", &payloads, Capture::default()));
    assert_eq!(app_node(&g).state, State::Working);
}

/// The app's turn (0–11), with its last reply (10) replaced by `text`, and
/// that reply and the turn's `stop` (11) in the order given.
fn turn_ending(text: &str, reply_first: bool, capture: Capture) -> Graph {
    let mut payloads = approvals()[..12].to_vec();
    payloads[10]["text"] = text.into();
    if !reply_first {
        payloads.swap(10, 11);
    }
    reduce(translate_as("cursor", &payloads, capture))
}

#[test]
fn a_turn_ending_on_a_question_needs_you_whichever_hook_lands_first() {
    for reply_first in [true, false] {
        let g = turn_ending(
            "Both ran. Shall I add them to a script?",
            reply_first,
            Capture::default(),
        );
        let node = app_node(&g);
        assert_eq!(
            node.state,
            State::InputRequired,
            "reply first: {reply_first}"
        );
        assert_eq!(node.attention.as_deref(), Some("Asks you a question"));
    }
    // Past closing formatting too.
    let g = turn_ending("**Want me to go on?**", true, Capture::default());
    assert_eq!(app_node(&g).state, State::InputRequired);
}

#[test]
fn a_turn_that_doesnt_end_on_a_question_is_idle() {
    for reply_first in [true, false] {
        let g = turn_ending("Both ran.", reply_first, Capture::default());
        assert_eq!(app_node(&g).state, State::Idle);
    }
    // The fixture's own reply.
    let g = reduce(translate_as(
        "cursor",
        &approvals()[..12],
        Capture::default(),
    ));
    assert_eq!(app_node(&g).state, State::Idle);
}

#[test]
fn the_question_itself_is_kept_only_with_body_capture() {
    let text = "Both ran.\n\nShall I add them to a script?";
    let g = turn_ending(text, true, Capture { bodies: true });
    assert_eq!(
        app_node(&g).attention.as_deref(),
        Some("Asks: Shall I add them to a script?")
    );
    let events = translate_as("cursor", &approvals()[10..11], Capture::default());
    assert!(!serde_json::to_string(&events).unwrap().contains("reply"));
}

#[test]
fn a_cli_run_waits_for_approval_and_goes_on() {
    // (Not from the CLI's settings here: `CURSOR_INVOKED_AS` isn't set in
    // tests, so this is the guess, as in the app.)
    let g = approvals_after(16, 20, Capture::default());
    let cli = format!(
        "cursor:{}",
        approvals()[12]["conversation_id"].as_str().unwrap()
    );
    assert_eq!(g.nodes[&cli].state, State::InputRequired);
    let g = approvals_after(17, 20, Capture::default());
    assert_eq!(g.nodes[&cli].state, State::Working);
    let g = approvals_after(21, 0, Capture::default());
    assert_eq!(g.nodes[&cli].state, State::Completed);
}

#[test]
fn the_cli_asks_unless_a_rule_names_the_command() {
    use agent_graph::adapter::cursor::asks;
    let rules = |r: &[&str]| r.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let allow = rules(&[
        "Shell(ls)",
        "Shell(git)",
        "Shell(curl:*)",
        "Shell(npm:run *)",
    ]);
    let deny = rules(&["Shell(rm)"]);
    assert_eq!(asks("ls -la", &allow, &deny), Some(false));
    assert_eq!(asks("git status", &allow, &deny), Some(false));
    assert_eq!(
        asks("curl -sI https://example.com", &allow, &deny),
        Some(false)
    );
    assert_eq!(asks("npm run build", &allow, &deny), Some(false));
    assert_eq!(asks("npm install", &allow, &deny), Some(true));
    assert_eq!(asks("date", &allow, &deny), Some(true));
    assert_eq!(
        asks("lsof -i", &allow, &deny),
        Some(true),
        "a word, not a prefix"
    );
    // Refused, not asked about.
    assert_eq!(asks("rm -rf build", &allow, &deny), Some(false));
    // Chains aren't settled by Cursor's docs: back to the guess.
    assert_eq!(asks("date && sleep 2 && echo done", &allow, &deny), None);
    assert_eq!(asks("ls | wc -l", &allow, &deny), None);
}

#[test]
fn commands_queued_together_are_each_tracked_until_each_has_run() {
    // Cursor runs `beforeShellExecution` for each command it proposes, at
    // once, then asks about them one at a time: here `date` (3) and the
    // second command (7) are both proposed, and `date` runs (4) while the
    // second is still waiting for you.
    let p = approvals();
    let payloads = vec![
        p[0].clone(),
        p[1].clone(),
        p[3].clone(),
        p[7].clone(),
        p[4].clone(),
    ];
    let events = translate_as("cursor", &payloads, Capture::default());
    let at = |waited: u64| {
        agent_graph::reducer::reduce(
            events.clone(),
            &agent_graph::reducer::Options {
                now: t0() + std::time::Duration::from_secs(4 + waited),
                stale_after: std::time::Duration::from_secs(30 * 60),
            },
        )
    };
    assert_eq!(app_node(&at(2)).state, State::Working);
    let g = at(20);
    assert_eq!(app_node(&g).state, State::InputRequired);
    assert_eq!(
        app_node(&g).attention.as_deref(),
        Some(agent_graph::reducer::MAY_ASK)
    );
    // The second runs: nothing's waiting.
    let mut payloads = payloads;
    payloads.push(p[8].clone());
    let g = reduce(translate_as("cursor", &payloads, Capture::default()));
    assert_eq!(app_node(&g).state, State::Working);
    // The turn's end, too, with any still pending.
    let g = reduce(translate_as(
        "cursor",
        &[p[0].clone(), p[1].clone(), p[3].clone(), p[11].clone()],
        Capture::default(),
    ));
    assert_eq!(app_node(&g).state, State::Idle);
}

#[test]
fn the_clis_flags_say_whether_it_asks() {
    use agent_graph::adapter::cursor::{RunMode, run_mode};
    let agent = "/Users/me/.local/share/cursor-agent/versions/2026.10.01/cursor-agent";
    assert_eq!(run_mode(&format!("{agent} --trust")), RunMode::Settings);
    assert_eq!(
        run_mode(&format!("{agent} --trust Run date")),
        RunMode::Settings
    );
    for flag in ["--force", "-f", "--yolo", "-p", "--print"] {
        assert_eq!(
            run_mode(&format!("{agent} --trust {flag} Run date")),
            RunMode::NeverAsks,
            "{flag}"
        );
    }
    assert_eq!(
        run_mode(&format!("{agent} --auto-review")),
        RunMode::Unknown
    );
}

/// A turn in Plan mode: payload 1 sent in Plan mode, ending (11) with the
/// reply (10), in either order.
fn plan_turn(reply_first: bool) -> Graph {
    let mut payloads = approvals()[..12].to_vec();
    payloads[1]["composer_mode"] = "plan".into();
    if !reply_first {
        payloads.swap(10, 11);
    }
    reduce(translate_as("cursor", &payloads, Capture::default()))
}

#[test]
fn a_turn_in_plan_mode_ends_on_a_plan_waiting_for_you() {
    for reply_first in [true, false] {
        let g = plan_turn(reply_first);
        assert_eq!(app_node(&g).state, State::InputRequired, "{reply_first}");
        assert_eq!(
            app_node(&g).attention.as_deref(),
            Some(agent_graph::reducer::PLAN_READY)
        );
    }
    // Building it is a new turn, in Agent mode, which ends idle.
    let mut payloads = approvals()[..12].to_vec();
    payloads[1]["composer_mode"] = "plan".into();
    payloads.extend(approvals()[1..12].iter().cloned());
    let g = reduce(translate_as("cursor", &payloads, Capture::default()));
    assert_eq!(app_node(&g).state, State::Idle);
}

#[test]
fn a_turn_cursor_calls_an_error_isnt_shown_as_one() {
    // The CLI says a turn stopped with Esc ended in an error.
    let mut stop = approvals()[11].clone();
    stop["status"] = "error".into();
    let g = reduce(translate_as(
        "cursor",
        &[approvals()[0].clone(), stop],
        Capture::default(),
    ));
    assert_eq!(app_node(&g).state, State::Idle);
    assert_eq!(app_node(&g).summary, None);
}

#[test]
fn a_cli_chats_name_is_in_its_meta_json() {
    use agent_graph::adapter::cursor::cli_title;
    let chats = tempfile::tempdir().unwrap();
    let id = "71159a97-8182-4380-b9d7-e817cd70427f";
    // Under a folder for the chat's working folder.
    let dir = chats
        .path()
        .join("14db98aa102fd4301b65d83d6cafbaf7")
        .join(id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("meta.json"),
        r#"{"schemaVersion":1,"hasConversation":true,"title":"Run Date Sleep","cwd":"/Users/dev/app"}"#,
    )
    .unwrap();
    assert_eq!(
        cli_title(chats.path(), id).as_deref(),
        Some("Run Date Sleep")
    );
    // A print run's has no title; an id that isn't one names no file.
    std::fs::write(dir.join("meta.json"), r#"{"schemaVersion":1}"#).unwrap();
    assert_eq!(cli_title(chats.path(), id), None);
    assert_eq!(cli_title(chats.path(), "../../etc"), None);
}

#[test]
fn an_app_chats_name_is_in_the_apps_database() {
    use agent_graph::adapter::cursor::app_titles;
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("state.vscdb");
    // A database shaped as Cursor's, kept open by a writer in WAL mode, as
    // Cursor keeps it: what it last wrote is still only in the -wal file.
    let writer = rusqlite::Connection::open(&store).unwrap();
    writer
        .execute_batch(
            "pragma journal_mode = wal; pragma wal_autocheckpoint = 0; \
             create table composerHeaders (composerId text primary key, value text); \
             insert into composerHeaders values \
             ('1cf069c4-2f45-49dd-acdb-6fd3f95ec3ed', '{\"name\":\"Runbook execution steps\"}'), \
             ('5d18b682-83e0-476d-a933-fd32497f2058', '{\"type\":\"head\"}');",
        )
        .unwrap();
    let all = app_titles(&store, None).unwrap();
    assert_eq!(all.len(), 1, "a chat with no name yet isn't listed");
    assert_eq!(
        all["1cf069c4-2f45-49dd-acdb-6fd3f95ec3ed"],
        "Runbook execution steps"
    );
    let one = app_titles(&store, Some("1cf069c4-2f45-49dd-acdb-6fd3f95ec3ed")).unwrap();
    assert_eq!(one.len(), 1);
    assert!(
        app_titles(&store, Some("5d18b682-83e0-476d-a933-fd32497f2058"))
            .unwrap()
            .is_empty()
    );
    // Nothing that isn't an id is looked up.
    assert_eq!(app_titles(&store, Some("x' or '1'='1")), None);
    // Renamed, while the writer still has it open.
    writer
        .execute(
            "update composerHeaders set value = '{\"name\":\"Stress test\"}' \
             where composerId = '1cf069c4-2f45-49dd-acdb-6fd3f95ec3ed'",
            [],
        )
        .unwrap();
    assert_eq!(
        app_titles(&store, None).unwrap()["1cf069c4-2f45-49dd-acdb-6fd3f95ec3ed"],
        "Stress test"
    );
}

/// A database shaped as Cursor's app's (see `an_app_chats_name_is_in_the_apps_database`),
/// with one chat whose plan is waiting and whose todo list is part done.
fn app_store_with_a_plan(dir: &std::path::Path) -> (std::path::PathBuf, rusqlite::Connection) {
    let store = dir.join("state.vscdb");
    let writer = rusqlite::Connection::open(&store).unwrap();
    writer
        .execute_batch(
            r#"pragma journal_mode = wal; pragma wal_autocheckpoint = 0;
            create table composerHeaders (composerId text primary key, value text);
            create table cursorDiskKV (key text primary key, value blob);
            insert into composerHeaders values ('927511cd-0000-4000-8000-000000000000',
              '{"name":"CHANGELOG.md planning","hasPendingPlan":true}');
            insert into cursorDiskKV values ('composerData:927511cd-0000-4000-8000-000000000000',
              '{"status":"completed","todos":[
                {"id":"1","content":"Survey the project","status":"completed","dependencies":[]},
                {"id":"2","content":"Write CHANGELOG.md","status":"in_progress","dependencies":[]},
                {"id":"3","content":"Link it from README","status":"pending","dependencies":[]},
                {"id":"4","content":"Tag a release","status":"cancelled","dependencies":[]}]}');"#,
        )
        .unwrap();
    (store, writer)
}

#[test]
fn an_app_chats_plan_and_todo_list_are_in_the_apps_database() {
    use agent_graph::adapter::cursor::app_chat;
    let dir = tempfile::tempdir().unwrap();
    let (store, writer) = app_store_with_a_plan(dir.path());
    let chat = app_chat(&store, "927511cd-0000-4000-8000-000000000000").unwrap();
    assert!(chat.plan_pending);
    let tasks: Vec<_> = chat
        .todos
        .iter()
        .map(|t| (t.text.as_str(), t.status))
        .collect();
    assert_eq!(
        tasks,
        [
            ("Survey the project", TaskStatus::Completed),
            ("Write CHANGELOG.md", TaskStatus::InProgress),
            ("Link it from README", TaskStatus::Pending),
            // Cancelled is done with, as far as the list goes.
            ("Tag a release", TaskStatus::Completed),
        ]
    );
    // Built: no plan waiting, as soon as Cursor saves it.
    writer
        .execute(
            "update composerHeaders set value = '{\"hasPendingPlan\":false}'",
            [],
        )
        .unwrap();
    assert!(
        !app_chat(&store, "927511cd-0000-4000-8000-000000000000")
            .unwrap()
            .plan_pending
    );
    // A chat it doesn't have (a CLI chat's): nothing.
    assert_eq!(
        app_chat(&store, "385fd5a6-3d88-4cc1-8ebd-a6883b0e4fbe"),
        None
    );
}

#[test]
fn the_viewer_shows_what_cursor_saved_on_its_chats() {
    use agent_graph::adapter::cursor::AppChat;
    let mut chats = std::collections::BTreeMap::new();
    let dir = tempfile::tempdir().unwrap();
    let (store, _writer) = app_store_with_a_plan(dir.path());
    let saved =
        agent_graph::adapter::cursor::app_chat(&store, "927511cd-0000-4000-8000-000000000000")
            .unwrap();
    let id = format!(
        "cursor:{}",
        approvals()[0]["conversation_id"].as_str().unwrap()
    );
    chats.insert(id.clone(), saved.clone());
    // The turn has ended: idle, with a plan waiting.
    let mut g = reduce(translate_as(
        "cursor",
        &approvals()[..12],
        Capture::default(),
    ));
    agent_graph::timeline::with_cursor(&mut g, &chats);
    let node = &g.nodes[&id];
    assert_eq!(node.state, State::InputRequired);
    assert_eq!(
        node.attention.as_deref(),
        Some(agent_graph::reducer::PLAN_READY)
    );
    assert_eq!(node.tasks.len(), 4);
    assert_eq!(node.open_tasks, 2);
    assert_eq!(
        node.headline.as_deref(),
        Some("Write CHANGELOG.md (+1 pending)")
    );
    // Still at work: not waiting on its plan yet.
    let mut g = reduce(translate_as(
        "cursor",
        &approvals()[..3],
        Capture::default(),
    ));
    agent_graph::timeline::with_cursor(&mut g, &chats);
    assert_eq!(g.nodes[&id].state, State::Working);
    // Nothing saved: as the log says.
    chats.insert(id.clone(), AppChat::default());
    let mut g = reduce(translate_as(
        "cursor",
        &approvals()[..12],
        Capture::default(),
    ));
    agent_graph::timeline::with_cursor(&mut g, &chats);
    assert_eq!(g.nodes[&id].state, State::Idle);
    assert!(g.nodes[&id].tasks.is_empty());
}

#[test]
fn archived_app_chats_are_listed_from_the_apps_database() {
    use agent_graph::adapter::cursor::archived_chats;
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("state.vscdb");
    let db = rusqlite::Connection::open(&store).unwrap();
    db.execute_batch(
        "create table composerHeaders (composerId text primary key, isArchived integer, value text); \
         insert into composerHeaders values ('6c1712be-0000-4000-8000-000000000000', 1, '{}'); \
         insert into composerHeaders values ('1cf069c4-2f45-49dd-acdb-6fd3f95ec3ed', 0, '{}');",
    )
    .unwrap();
    let archived = archived_chats(&store).unwrap();
    assert_eq!(
        archived.into_iter().collect::<Vec<_>>(),
        ["6c1712be-0000-4000-8000-000000000000"]
    );
}

/// A tool call's message, as Cursor's app saves it (only what's read).
fn tool_message(chat: &str, message: &str, tool: &serde_json::Value) -> (String, String) {
    (
        format!("bubbleId:{chat}:{message}"),
        serde_json::json!({"type": 2, "toolFormerData": tool}).to_string(),
    )
}

#[test]
fn what_an_app_chat_is_waiting_on_you_for_is_in_its_saved_tool_calls() {
    use agent_graph::adapter::cursor::{Waiting, app_chat};
    let dir = tempfile::tempdir().unwrap();
    let (store, writer) = app_store_with_a_plan(dir.path());
    let chat = "927511cd-0000-4000-8000-000000000000";
    let put = |message: &str, tool: serde_json::Value| {
        let (key, value) = tool_message(chat, message, &tool);
        writer
            .execute(
                "insert or replace into cursorDiskKV values (?1, ?2)",
                [key, value],
            )
            .unwrap();
    };
    // Nothing waiting: a command that ran.
    put(
        "a",
        serde_json::json!({"name": "run_terminal_command_v2", "status": "completed",
            "additionalData": {"status": "success"}}),
    );
    // Another chat's request isn't this one's.
    let (key, value) = tool_message(
        "927511cd-0000-4000-8000-000000000001",
        "z",
        &serde_json::json!({"name": "run_terminal_command_v2", "status": "loading",
            "additionalData": {"reviewData": {"status": "Requested"}}}),
    );
    writer
        .execute("insert into cursorDiskKV values (?1, ?2)", [key, value])
        .unwrap();
    // A plan, made and built: its review stays `Requested`.
    put(
        "p",
        serde_json::json!({"name": "create_plan", "status": "completed",
            "additionalData": {"reviewData": {"status": "Requested", "selectedOption": "none"}}}),
    );
    let read = || app_chat(&store, chat).unwrap();
    assert_eq!(read().waiting, None);
    assert!(read().knows_waits);
    // Asking to run a command, as Cursor saves it when it asks.
    put(
        "b",
        serde_json::json!({"name": "run_terminal_command_v2", "status": "loading",
            "additionalData": {"status": "cancelled", "blockReason": "Shell allowlist is empty",
                "reviewData": {"status": "Requested", "selectedOption": "rejectAndTellWhatToDoDifferently",
                    "approvalType": "user"}}}),
    );
    assert_eq!(read().waiting, Some(Waiting::Approval));
    // Approved.
    put(
        "b",
        serde_json::json!({"name": "run_terminal_command_v2", "status": "completed",
            "additionalData": {"status": "success",
                "reviewData": {"status": "Done", "selectedOption": "run", "approvalType": "user"}}}),
    );
    assert_eq!(read().waiting, None);
    // A question, shown.
    put(
        "c",
        serde_json::json!({"name": "ask_question", "status": "loading"}),
    );
    assert_eq!(read().waiting, Some(Waiting::Question));
    // Answered.
    put(
        "c",
        serde_json::json!({"name": "ask_question", "status": "completed", "userDecision": "accepted",
            "additionalData": {"status": "submitted"}}),
    );
    assert_eq!(read().waiting, None);
}

#[test]
fn the_viewer_shows_what_an_app_chat_waits_for_and_drops_the_guess() {
    use agent_graph::adapter::cursor::{AppChat, Waiting};
    let id = format!(
        "cursor:{}",
        approvals()[0]["conversation_id"].as_str().unwrap()
    );
    let with = |waiting: Option<Waiting>, knows_waits: bool, n: usize, waited: u64| {
        let mut chats = std::collections::BTreeMap::new();
        chats.insert(
            id.clone(),
            AppChat {
                waiting,
                knows_waits,
                ..Default::default()
            },
        );
        let mut g = approvals_after(n, waited, Capture::default());
        agent_graph::timeline::with_cursor(&mut g, &chats);
        let node = &g.nodes[&id];
        (node.state, node.attention.clone())
    };
    // A command proposed (payload 3), 20 s ago, that the app's database says
    // is waiting for approval: for sure, and said so.
    assert_eq!(
        with(Some(Waiting::Approval), true, 4, 20),
        (
            State::InputRequired,
            Some("Waiting for your approval".to_string())
        )
    );
    // That it says isn't: running, slowly, not the guess.
    assert_eq!(with(None, true, 4, 20), (State::Working, None));
    // Where it can't say (a CLI chat): the guess stands.
    assert_eq!(
        with(None, false, 4, 20),
        (
            State::InputRequired,
            Some(agent_graph::reducer::MAY_ASK.to_string())
        )
    );
    // A question, at work.
    assert_eq!(
        with(Some(Waiting::Question), true, 2, 1).1.as_deref(),
        Some("Asks you a question")
    );
    // The turn's over (payload 11): a request left over isn't waiting.
    assert_eq!(with(Some(Waiting::Approval), true, 12, 1).0, State::Idle);
}
