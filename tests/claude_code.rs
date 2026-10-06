//! The Claude Code adapter and reducer together, driven by recorded-shape
//! hook payloads in `tests/fixtures/claude-code/session.jsonl`.

mod common;

use agent_graph::adapter::{self, Capture};
use agent_graph::event::{Payload, State, TaskStatus};
use agent_graph::reducer::NodeKind;
use common::*;
use serde_json::json;

#[test]
fn session_start_creates_an_idle_session() {
    let g = session_after(1);
    let s = &g.nodes[SESSION];
    assert_eq!(s.kind, NodeKind::Session);
    assert_eq!(s.state, State::Idle);
    assert_eq!(s.cwd.as_deref(), Some("/home/dev/app"));
    assert_eq!(g.roots, vec![SESSION.to_string()]);
}

#[test]
fn incremental_task_updates_merge() {
    let g = session_after(5);
    let s = &g.nodes[SESSION];
    assert_eq!(s.state, State::Working);
    let statuses: Vec<_> = s.tasks.iter().map(|t| (t.id.as_str(), t.status)).collect();
    assert_eq!(
        statuses,
        [("1", TaskStatus::InProgress), ("2", TaskStatus::Pending)]
    );
    assert_eq!(s.open_tasks, 2);
    assert_eq!(
        s.headline.as_deref(),
        Some("Finding the auth code (+1 pending)")
    );
}

#[test]
fn foreground_spawn_blocks_until_the_child_starts_and_returns() {
    // Requested, not started yet.
    let g = session_after(6);
    let blocked = g.nodes[SESSION].blocked.as_ref().expect("blocked");
    assert_eq!(blocked.starting, 1);
    assert!(blocked.on.is_empty());

    // Started: the wait is bound to the child, which inherits the purpose.
    let g = session_after(7);
    let child = &g.nodes[&agent("a1f00d")];
    assert_eq!(child.parent.as_deref(), Some(SESSION));
    assert_eq!(child.purpose.as_deref(), Some("Find the auth middleware"));
    assert_eq!(child.agent_type.as_deref(), Some("Explore"));
    assert_eq!(child.background, Some(false));
    let blocked = g.nodes[SESSION].blocked.as_ref().expect("blocked");
    assert_eq!(blocked.on, vec![agent("a1f00d")]);
    assert_eq!(blocked.nodes, 1);

    // Returned: no longer blocked, child finished, binding confirmed.
    let g = session_after(9);
    assert!(g.nodes[SESSION].blocked.is_none());
    let child = &g.nodes[&agent("a1f00d")];
    assert_eq!(child.state, State::Completed);
    assert_eq!(child.spawned_by.as_deref(), Some("toolu_A"));
}

#[test]
fn background_spawn_does_not_block() {
    let g = session_after(12);
    assert!(g.nodes[SESSION].blocked.is_none());
    let child = &g.nodes[&agent("b2c0de")];
    assert_eq!(child.background, Some(true));
    assert_eq!(child.purpose.as_deref(), Some("Run the test suite"));
    assert_eq!(child.state, State::Working);
    assert_eq!(
        g.nodes[SESSION].children,
        vec![agent("a1f00d"), agent("b2c0de")]
    );
}

#[test]
fn messages_are_recorded_on_both_sides() {
    let g = session_after(13);
    let sent = &g.nodes[SESSION].messages;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].peer, agent("b2c0de"));
    assert_eq!(sent[0].summary.as_deref(), Some("Also run lint"));
    assert_eq!(sent[0].body, None, "bodies are off by default");
    let received = &g.nodes[&agent("b2c0de")].messages;
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].peer, SESSION);
}

#[test]
fn permission_prompt_needs_the_user_until_work_resumes() {
    let g = session_after(14);
    let s = &g.nodes[SESSION];
    assert_eq!(s.state, State::InputRequired);
    assert_eq!(
        s.attention.as_deref(),
        Some("Claude needs your permission to use Bash")
    );

    let g = session_after(15);
    let s = &g.nodes[SESSION];
    assert_eq!(s.state, State::Working);
    assert_eq!(s.attention, None);
    assert_eq!(s.tasks[0].status, TaskStatus::Completed);
}

#[test]
fn stop_goes_idle_and_session_end_completes() {
    let g = session_after(16);
    let s = &g.nodes[SESSION];
    assert_eq!(s.state, State::Idle);
    assert_eq!(s.headline.as_deref(), Some("1 task pending"));

    let g = session_after(18);
    assert_eq!(g.nodes[SESSION].state, State::Completed);
    assert_eq!(g.nodes[&agent("b2c0de")].state, State::Completed);
    assert!(g.nodes.values().all(|n| !n.stale));
}

/// Real payloads captured from Claude Code 2.1.118 (`claude -p`, Haiku),
/// with paths scrubbed.
#[test]
fn live_capture_builds_the_expected_graph() {
    let payloads = fixture("claude-code/live-2.1.118-todowrite.jsonl");
    let events = translate(&payloads, Capture::default());
    assert!(
        events.iter().all(|e| e.kind != "unknown"),
        "every hook was understood"
    );

    let g = reduce(events);
    let session = "claude-code:fe02378e-9928-4342-9206-2afd53442745";
    let child = format!("{session}/a503dc4ad7b3addb3");
    assert_eq!(g.roots, vec![session.to_string()]);
    let s = &g.nodes[session];
    assert_eq!(s.state, State::Completed);
    assert_eq!(s.cwd.as_deref(), Some("/home/dev/project"));
    assert_eq!(s.children, vec![child.clone()]);
    assert_eq!((s.tasks.len(), s.open_tasks), (2, 0));
    assert!(s.blocked.is_none());
    let c = &g.nodes[&child];
    assert_eq!(c.state, State::Completed);
    assert_eq!(c.agent_type.as_deref(), Some("Explore"));
    assert_eq!(c.purpose.as_deref(), Some("List the text files"));
    assert_eq!(
        c.spawned_by.as_deref(),
        Some("toolu_01K99xWq6JaWBTejSD3iaR7C")
    );
}

#[test]
fn bodies_are_only_kept_when_capture_is_on() {
    let payloads = fixture("claude-code/session.jsonl");
    let find = |capture| {
        translate(&payloads, capture)
            .into_iter()
            .filter_map(|e| match e.payload() {
                Payload::MessageSent(m) => Some(m.body),
                Payload::AgentFinished(f) => Some(f.summary),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert!(find(Capture { bodies: false }).iter().all(Option::is_none));
    assert!(find(Capture { bodies: true }).iter().all(Option::is_some));
}

#[test]
fn nested_spawn_moves_the_child_under_its_requester() {
    let payloads = vec![
        json!({"session_id": "s", "hook_event_name": "SessionStart"}),
        json!({"session_id": "s", "hook_event_name": "SubagentStart", "agent_id": "outer", "agent_type": "Plan"}),
        // A hook inside `outer` asks for another agent...
        json!({"session_id": "s", "agent_id": "outer", "hook_event_name": "PreToolUse", "tool_name": "Agent",
               "tool_use_id": "toolu_N", "tool_input": {"description": "Dig deeper", "subagent_type": "Explore"}}),
        // ...but SubagentStart only knows the session.
        json!({"session_id": "s", "hook_event_name": "SubagentStart", "agent_id": "inner", "agent_type": "Explore"}),
    ];
    let g = reduce(translate(&payloads, Capture::default()));
    assert_eq!(
        g.nodes["claude-code:s/inner"].parent.as_deref(),
        Some("claude-code:s/outer")
    );
    assert_eq!(
        g.nodes["claude-code:s/outer"].children,
        vec!["claude-code:s/inner".to_string()]
    );
    assert!(
        g.nodes["claude-code:s"]
            .children
            .iter()
            .all(|c| c != "claude-code:s/inner")
    );
}

#[test]
fn a_wrong_guess_is_corrected_by_spawn_returned() {
    let pre = |call: &str, what: &str| {
        json!({"session_id": "s", "hook_event_name": "PreToolUse", "tool_name": "Agent", "tool_use_id": call,
               "tool_input": {"description": what, "subagent_type": "Explore"}})
    };
    let start = |id: &str| json!({"session_id": "s", "hook_event_name": "SubagentStart", "agent_id": id, "agent_type": "Explore"});
    let post = |call: &str, id: &str| {
        json!({"session_id": "s", "hook_event_name": "PostToolUse", "tool_name": "Agent", "tool_use_id": call,
               "tool_response": {"agentId": id, "status": "completed"}})
    };
    // Two parallel Explore agents; the second one starts first, so the guess
    // pairs them the wrong way round until each call returns.
    let payloads = vec![
        pre("call_x", "Look at X"),
        pre("call_y", "Look at Y"),
        start("ay"),
        start("ax"),
    ];
    let g = reduce(translate(&payloads, Capture::default()));
    assert_eq!(
        g.nodes["claude-code:s/ay"].purpose.as_deref(),
        Some("Look at X"),
        "the guess"
    );

    let mut payloads = payloads;
    payloads.push(post("call_x", "ax"));
    payloads.push(post("call_y", "ay"));
    let g = reduce(translate(&payloads, Capture::default()));
    assert_eq!(
        g.nodes["claude-code:s/ax"].spawned_by.as_deref(),
        Some("call_x")
    );
    assert_eq!(
        g.nodes["claude-code:s/ay"].spawned_by.as_deref(),
        Some("call_y")
    );
    assert_eq!(
        g.nodes["claude-code:s/ax"].purpose.as_deref(),
        Some("Look at X")
    );
    assert_eq!(
        g.nodes["claude-code:s/ay"].purpose.as_deref(),
        Some("Look at Y")
    );
    let spawns = &g.nodes["claude-code:s"].spawns;
    assert_eq!(spawns[0].child.as_deref(), Some("claude-code:s/ax"));
    assert_eq!(spawns[1].child.as_deref(), Some("claude-code:s/ay"));
}

#[test]
fn todo_write_replaces_the_list() {
    let todo = |items: serde_json::Value| json!({"session_id": "s", "hook_event_name": "PostToolUse", "tool_name": "TodoWrite", "tool_input": {"todos": items}});
    let payloads = vec![
        todo(
            json!([{"content": "A", "status": "pending", "activeForm": "Doing A"},
                    {"content": "B", "status": "pending", "activeForm": "Doing B"}]),
        ),
        todo(json!([{"content": "A", "status": "completed", "activeForm": "Doing A"}])),
    ];
    let g = reduce(translate(&payloads, Capture::default()));
    let tasks = &g.nodes["claude-code:s"].tasks;
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].status, TaskStatus::Completed);
}

#[test]
fn unknown_hooks_are_recorded_not_rejected() {
    let adapter = adapter::by_name("claude-code").unwrap();
    let t = adapter
        .translate(
            &json!({"session_id": "s", "hook_event_name": "SomethingNew"}),
            Capture::default(),
        )
        .unwrap();
    assert_eq!(t.drafts.len(), 1);
    assert_eq!(t.drafts[0].payload.type_name(), "unknown");

    // Hooks we match but don't need produce nothing.
    let t = adapter
        .translate(
            &json!({"session_id": "s", "hook_event_name": "PostToolUse", "tool_name": "Read"}),
            Capture::default(),
        )
        .unwrap();
    assert!(t.drafts.is_empty());
}

#[test]
fn payload_without_session_is_an_error() {
    let adapter = adapter::by_name("claude-code").unwrap();
    assert!(
        adapter
            .translate(&json!({"hook_event_name": "Stop"}), Capture::default())
            .is_err()
    );
}

#[test]
fn questions_and_plan_approval_need_the_user() {
    let ask = json!({"session_id": "s", "hook_event_name": "PreToolUse", "tool_name": "AskUserQuestion",
        "tool_use_id": "toolu_q", "tool_input": {"questions": [
            {"question": "Which database should the cache use?", "header": "Cache", "options": [], "multiSelect": false}]}});
    let answered = json!({"session_id": "s", "hook_event_name": "PostToolUse", "tool_name": "AskUserQuestion",
        "tool_use_id": "toolu_q", "tool_input": {}, "tool_response": {"answers": {}, "questions": []}});
    let plan = json!({"session_id": "s", "agent_id": "p1", "hook_event_name": "PreToolUse", "tool_name": "ExitPlanMode",
        "tool_use_id": "toolu_p", "tool_input": {"plan": "1. Do it"}});

    let g = reduce(translate(std::slice::from_ref(&ask), Capture::default()));
    let s = &g.nodes["claude-code:s"];
    assert_eq!(s.state, State::InputRequired);
    assert_eq!(
        s.attention.as_deref(),
        Some("Asks: Which database should the cache use?")
    );

    let g = reduce(translate(&[ask, answered], Capture::default()));
    assert_eq!(g.nodes["claude-code:s"].state, State::Working);

    let g = reduce(translate(&[plan], Capture::default()));
    let agent = &g.nodes["claude-code:s/p1"];
    assert_eq!(agent.state, State::InputRequired);
    assert_eq!(
        agent.attention.as_deref(),
        Some("Plan ready for your review")
    );
}

#[test]
fn a_turn_that_leaves_a_background_command_running_is_still_working() {
    // Transcript lines as Claude Code writes them: the command's start, then
    // the notification that wakes the session when it finishes.
    let started = json!({"type": "user", "message": {"role": "user", "content": [{"type": "tool_result",
        "tool_use_id": "toolu_b", "content": "Command running in background with ID: bzk"}]},
        "toolUseResult": {"stdout": "", "stderr": "", "interrupted": false, "backgroundTaskId": "bzk"}});
    let finished = json!({"type": "user", "origin": {"kind": "task-notification"}, "message": {"role": "user",
        "content": "<task-notification>\n<task-id>bzk</task-id>\n<status>completed</status>\n</task-notification>"}});
    let dir = tempfile::tempdir().unwrap();
    let transcript = dir.path().join("s.jsonl");
    let stop = |agent: Option<&str>| {
        let mut stop = json!({"session_id": "s", "hook_event_name": "Stop",
            "transcript_path": transcript.to_str().unwrap()});
        if let Some(agent) = agent {
            stop["agent_id"] = json!(agent);
        }
        let g = reduce(translate(&[stop], Capture::default()));
        g.nodes[&agent.map_or("claude-code:s".to_string(), |a| {
            format!("claude-code:s/{a}")
        })]
            .state
    };

    std::fs::write(&transcript, format!("{started}\n")).unwrap();
    assert_eq!(
        adapter::claude_code::background_commands_running(
            &std::fs::read_to_string(&transcript).unwrap()
        ),
        1
    );
    assert_eq!(stop(None), State::Working);
    // A subagent's turn ends as usual; the command isn't its.
    assert_eq!(stop(Some("a1")), State::Idle);

    std::fs::write(&transcript, format!("{started}\n{finished}\n")).unwrap();
    assert_eq!(stop(None), State::Idle);

    // No transcript to read: the turn is over.
    std::fs::remove_file(&transcript).unwrap();
    assert_eq!(stop(None), State::Idle);
}

#[test]
fn sessions_take_the_name_claude_code_gives_them() {
    let dir = tempfile::tempdir().unwrap();
    let transcript = dir.path().join("s.jsonl");
    let named = |title: &str| {
        json!({"type": "custom-title", "customTitle": title, "sessionId": "s"}).to_string() + "\n"
    };
    let hook = |name: &str| {
        json!({"session_id": "s", "hook_event_name": name,
            "transcript_path": transcript.to_str().unwrap()})
    };

    // Not named yet: the viewer falls back as before.
    std::fs::write(&transcript, "").unwrap();
    let g = reduce(translate(
        &[hook("SessionStart"), hook("UserPromptSubmit")],
        Capture::default(),
    ));
    assert_eq!(g.nodes["claude-code:s"].title, None);

    // Named after the first prompt, then renamed: the latest name wins.
    std::fs::write(
        &transcript,
        named("Fix the login bug") + &named("Login redirect loop"),
    )
    .unwrap();
    let g = reduce(translate(
        &[hook("SessionStart"), hook("Stop")],
        Capture::default(),
    ));
    assert_eq!(
        g.nodes["claude-code:s"].title.as_deref(),
        Some("Login redirect loop")
    );

    // A resumed session starts with its name, and an untitled status keeps it.
    let g = reduce(translate(
        &[
            hook("SessionStart"),
            json!({"session_id": "s", "hook_event_name": "Stop"}),
        ],
        Capture::default(),
    ));
    assert_eq!(
        g.nodes["claude-code:s"].title.as_deref(),
        Some("Login redirect loop")
    );
    assert_eq!(
        agent_graph::render::card_name(&g.nodes["claude-code:s"]),
        "Login redirect loop"
    );
}

#[test]
fn a_session_the_desktop_app_named_has_its_name_from_its_start() {
    let dir = tempfile::tempdir().unwrap();
    let transcript = dir.path().join("s.jsonl");
    let hook = |name: &str| {
        json!({"session_id": "s", "hook_event_name": name,
            "transcript_path": transcript.to_str().unwrap()})
    };
    // The app writes the name beside the transcript as it starts the
    // session; the transcript doesn't have it yet.
    std::fs::write(&transcript, "").unwrap();
    std::fs::create_dir(dir.path().join("s")).unwrap();
    std::fs::write(
        dir.path().join("s/custom-title.json"),
        json!({"customTitle": "Fix the hook"}).to_string(),
    )
    .unwrap();
    let events = translate(&[hook("SessionStart")], Capture::default());
    match events[0].payload() {
        Payload::SessionStarted(d) => assert_eq!(d.title.as_deref(), Some("Fix the hook")),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        agent_graph::adapter::claude_code::title_of(&transcript).as_deref(),
        Some("Fix the hook")
    );

    // Renamed since: the transcript's name wins.
    std::fs::write(
        &transcript,
        json!({"type": "custom-title", "customTitle": "Hook paths"}).to_string() + "\n",
    )
    .unwrap();
    let g = reduce(translate(
        &[hook("SessionStart"), hook("Stop")],
        Capture::default(),
    ));
    assert_eq!(
        g.nodes["claude-code:s"].title.as_deref(),
        Some("Hook paths")
    );
}

#[test]
fn a_suggested_session_is_paired_by_its_name() {
    let dir = tempfile::tempdir().unwrap();
    let hook = |session: &str, name: &str, title: Option<&str>| {
        let transcript = dir.path().join(format!("{session}.jsonl"));
        let line = title.map_or(String::new(), |t| {
            json!({"type": "custom-title", "customTitle": t}).to_string() + "\n"
        });
        std::fs::write(&transcript, line).unwrap();
        json!({"session_id": session, "hook_event_name": name,
            "transcript_path": transcript.to_str().unwrap()})
    };
    let suggest = |call: &str, title: &str| {
        json!({"session_id": "p", "hook_event_name": "PostToolUse",
            "tool_name": "mcp__ccd_session__spawn_task", "tool_use_id": call,
            "tool_input": {"title": title, "tldr": "Stray root files in worktree commits.",
                "prompt": "…"},
            "tool_response": [{"type": "text", "text": "task_id: t1"}]})
    };
    let payloads = [
        hook("p", "SessionStart", Some("Publishing")),
        // Named so before it was suggested: not it.
        hook("early", "SessionStart", None),
        hook("early", "Stop", Some("Fix the hook")),
        suggest("toolu_S", "Fix the hook"),
        hook("p", "Stop", Some("Publishing")),
        // Another session, by another name.
        hook("other", "SessionStart", None),
        hook("other", "Stop", Some("Something else")),
        // Accepted, much later; named once its first turn is over.
        hook("c", "SessionStart", None),
        hook("c", "UserPromptSubmit", None),
        hook("c", "Stop", Some("Fix the hook")),
        // The same name again: the request is taken.
        hook("again", "SessionStart", None),
        hook("again", "Stop", Some("Fix the hook")),
    ];
    let events = translate(&payloads, Capture::default());
    let g = reduce(events.clone());

    let child = &g.nodes["claude-code:c"];
    assert_eq!(child.parent.as_deref(), Some("claude-code:p"));
    assert_eq!(child.spawned_by.as_deref(), Some("toolu_S"));
    assert_eq!(child.link.as_deref(), Some("suggested"));
    assert_eq!(
        child.purpose.as_deref(),
        Some("Stray root files in worktree commits.")
    );
    assert_eq!(child.background, Some(true));
    for id in ["early", "other", "again"] {
        assert_eq!(g.nodes[&format!("claude-code:{id}")].parent, None, "{id}");
    }
    // Nothing waits on a suggestion.
    assert!(g.nodes["claude-code:p"].blocked.is_none());
    let request = events.iter().find(|e| e.kind == "spawn.requested").unwrap();
    assert_eq!(
        agent_graph::timeline::describe(request, &g).1,
        "Suggested a session: Fix the hook"
    );
}

#[test]
fn a_suggestion_waits_for_its_name() {
    // A session linked to the one that suggested it, but started some other
    // way (a script it ran, say) and by another name, isn't the suggested
    // one, though it's the only request it might answer.
    let mut events = translate(
        &[
            json!({"session_id": "p", "hook_event_name": "SessionStart"}),
            json!({"session_id": "p", "hook_event_name": "PostToolUse",
                "tool_name": "mcp__ccd_session__spawn_task", "tool_use_id": "toolu_S",
                "tool_input": {"title": "Fix the hook"}}),
            json!({"session_id": "launched", "hook_event_name": "SessionStart"}),
        ],
        Capture::default(),
    );
    events[2].parent = Some("claude-code:p".into());
    let g = reduce(events);
    let launched = &g.nodes["claude-code:launched"];
    assert_eq!(launched.parent.as_deref(), Some("claude-code:p"));
    assert_eq!(launched.spawned_by, None);
    assert_eq!(g.nodes["claude-code:p"].spawns[0].child, None);
}

/// A helper Claude Code runs on its own between turns (a prompt suggestion,
/// say) has no type and no transcript, and is never said to start: it's
/// counted on its session, not drawn. A subagent whose start was missed,
/// with its type, still is.
#[test]
fn background_helpers_are_counted_not_drawn() {
    let stop = |agent: &str, agent_type: &str| {
        json!({
            "session_id": "5f2c1e8a-3b4d-4e5f-8a9b-0c1d2e3f4a5b",
            "transcript_path": "/home/dev/.claude/projects/-home-dev-app/5f2c1e8a-3b4d-4e5f-8a9b-0c1d2e3f4a5b.jsonl",
            "cwd": "/home/dev/app",
            "hook_event_name": "SubagentStop",
            "stop_hook_active": false,
            "agent_id": agent,
            "agent_type": agent_type,
            "agent_transcript_path": format!("/home/dev/.claude/projects/-home-dev-app/5f2c1e8a-3b4d-4e5f-8a9b-0c1d2e3f4a5b/subagents/agent-{agent}.jsonl"),
            "last_assistant_message": "run the tests"
        })
    };
    let payloads = vec![
        fixture("claude-code/session.jsonl").remove(0),
        stop("aa63958c", ""),
        stop("ab12cd34", ""),
        stop("ac0ffee1", "Explore"),
    ];
    let events = translate(&payloads, Capture::default());
    let flags: Vec<_> = events
        .iter()
        .filter_map(|e| match e.payload() {
            Payload::AgentFinished(d) => d.background,
            _ => None,
        })
        .collect();
    assert_eq!(flags, [true, true, false]);
    let g = reduce(events);
    assert_eq!(g.nodes[SESSION].background_agents, 2);
    assert_eq!(g.nodes[SESSION].children, [agent("ac0ffee1")]);
}
