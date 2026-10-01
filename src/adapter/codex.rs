//! Codex CLI hooks → Agent Graph events.
//!
//! Codex's hooks are modelled on Claude Code's: every payload carries
//! `session_id` and `hook_event_name`, and tool payloads carry `tool_name`,
//! `tool_input`, `tool_use_id` and (after the tool runs) `tool_response`.
//! Unlike Claude Code's, `session_id` is always the root thread's, even
//! inside a subagent, where `agent_id` is the subagent's own thread id.
//! SessionStart, SessionEnd, Stop and Interrupt never fire in a subagent;
//! SubagentStart and SubagentStop do instead. See docs/codex.md.

use serde_json::Value;

use super::claude_code::read_file;
use super::{Adapter, Capture, Draft, Translation, shell, str_at};
use crate::event::{
    AgentFinished, AgentSpawned, FinishStatus, Payload, SessionEnded, SessionStarted, SpawnKind,
    SpawnRequested, SpawnReturned, State, Status, TaskItem, TaskStatus, TasksUpdated, WaitEnded,
    WaitStarted, truncate_chars,
};
use crate::paths::file_key;

pub const PROVIDER: &str = "codex";

/// Longest label we keep (a plan step, a question, a subagent's nickname).
const LABEL_MAX: usize = 200;
/// Longest body we keep when body capture is on.
const BODY_MAX: usize = 2000;

pub struct Codex;

pub fn node_id(session: &str, agent: Option<&str>) -> String {
    match agent {
        Some(agent) => format!("{PROVIDER}:{session}/{agent}"),
        None => format!("{PROVIDER}:{session}"),
    }
}

impl Adapter for Codex {
    fn provider(&self) -> &'static str {
        PROVIDER
    }

    fn adapter_id(&self) -> &'static str {
        "codex@1"
    }

    fn translate(&self, input: &Value, capture: Capture) -> Result<Translation, String> {
        let session = str_at(input, &["session_id"]).ok_or("payload has no session_id")?;
        let session_node = node_id(session, None);
        // A subagent's own thread, when the hook fired in one.
        let agent_id = str_at(input, &["agent_id"]).filter(|a| *a != session);
        let node = node_id(session, agent_id);
        let hook = str_at(input, &["hook_event_name"]).unwrap_or("");
        let label = |path: &[&str]| str_at(input, path).map(|s| truncate_chars(s, LABEL_MAX));

        let mut drafts = Vec::new();
        match hook {
            "SessionStart" => drafts.push(Draft::new(
                &session_node,
                Payload::SessionStarted(SessionStarted {
                    cwd: str_at(input, &["cwd"]).map(String::from),
                    source: str_at(input, &["source"]).map(String::from),
                    // A resumed session already has its name.
                    title: title_of(session),
                    transcript_path: str_at(input, &["transcript_path"]).map(String::from),
                    ..Default::default()
                }),
            )),
            "SessionEnd" => drafts.push(Draft::new(
                &session_node,
                Payload::SessionEnded(SessionEnded {
                    reason: str_at(input, &["reason"]).map(String::from),
                }),
            )),
            // The session's name comes from its first prompt, during the
            // first turn (in the TUI; `codex exec` names none), so it's
            // looked for at each turn's start and end.
            "UserPromptSubmit" => drafts.push(match agent_id {
                Some(_) => status(&node, State::Working, None),
                None => titled(&session_node, session, State::Working),
            }),
            // A Plan-mode turn that ended with a plan waits for you to
            // approve it ("Implement this plan?"), which no hook says.
            "Stop" if plan_proposed(input) => drafts.push(Draft::new(
                &session_node,
                Payload::Status(Status {
                    state: State::InputRequired,
                    summary: Some("Plan ready for your review".to_string()),
                    title: title_of(session),
                }),
            )),
            // A turn that ended on a question nobody's answered (Codex's
            // cloud ends the turn there, and waits for your reply) still
            // waits for you.
            "Stop" if open_question(session, &session_node).is_some() => drafts.push(Draft::new(
                &session_node,
                Payload::Status(Status {
                    state: State::InputRequired,
                    summary: open_question(session, &session_node),
                    title: title_of(session),
                }),
            )),
            // A turn that ends by asking you something in words waits for
            // your reply as much as one that used the question tool.
            "Stop" if asks_in_words(input).is_some() => drafts.push(Draft::new(
                &session_node,
                Payload::Status(Status {
                    state: State::InputRequired,
                    // What it asks is the agent's own words: kept only with
                    // bodies.
                    summary: Some(
                        capture
                            .bodies
                            .then(|| asks_in_words(input))
                            .flatten()
                            .map(|q| truncate_chars(&format!("Asks: {q}"), LABEL_MAX))
                            .unwrap_or_else(|| "Asks you a question".to_string()),
                    ),
                    title: title_of(session),
                }),
            )),
            "Stop" | "Interrupt" => drafts.push(titled(&session_node, session, State::Idle)),
            "SubagentStart" => {
                let agent = agent_id.ok_or("SubagentStart has no agent_id")?;
                // Its rollout says which thread started it, for an agent
                // started by another agent, and the nickname Codex shows.
                let meta = str_at(input, &["transcript_path"])
                    .and_then(|p| session_meta(std::path::Path::new(p)));
                let parent = meta
                    .as_ref()
                    .and_then(|m| {
                        str_at(
                            m,
                            &["source", "subagent", "thread_spawn", "parent_thread_id"],
                        )
                        .or_else(|| str_at(m, &["parent_thread_id"]))
                    })
                    .filter(|p| *p != session && *p != agent)
                    .map_or_else(|| session_node.clone(), |p| node_id(session, Some(p)));
                // Its task's name (multi-agent v2), as the app shows it.
                let purpose = meta.as_ref().and_then(|m| {
                    str_at(m, &["agent_path"])
                        .or_else(|| {
                            str_at(m, &["source", "subagent", "thread_spawn", "agent_path"])
                        })
                        .and_then(task_label)
                });
                drafts.push(
                    Draft::new(
                        &node,
                        Payload::AgentSpawned(AgentSpawned {
                            agent_type: str_at(input, &["agent_type"]).map(String::from),
                            purpose,
                            ..Default::default()
                        }),
                    )
                    .with_parent(parent),
                );
                if let Some(nickname) = meta.as_ref().and_then(|m| {
                    str_at(m, &["agent_nickname"]).or_else(|| {
                        str_at(m, &["source", "subagent", "thread_spawn", "agent_nickname"])
                    })
                }) {
                    drafts.push(Draft::new(
                        &node,
                        Payload::Status(Status {
                            state: State::Working,
                            summary: None,
                            title: Some(truncate_chars(nickname, LABEL_MAX)),
                        }),
                    ));
                }
            }
            "SubagentStop" => {
                if agent_id.is_none() {
                    return Err("SubagentStop has no agent_id".into());
                }
                let summary = capture
                    .bodies
                    .then(|| str_at(input, &["last_assistant_message"]))
                    .flatten()
                    .map(|s| truncate_chars(s, BODY_MAX));
                drafts.push(Draft::new(
                    &node,
                    Payload::AgentFinished(AgentFinished {
                        status: FinishStatus::Completed,
                        summary,
                    }),
                ));
            }
            // Approval is needed, from you or Codex's auto-reviewer. Nothing
            // says when it's given: the node's next event clears it.
            "PermissionRequest" => {
                // Codex's own description of the action, if it gave one:
                // never the command itself, which can hold anything.
                let summary = match label(&["tool_input", "description"]) {
                    Some(what) => format!("Needs approval: {what}"),
                    None => match tool_name(input) {
                        "Bash" => "Needs approval to run a command".to_string(),
                        "apply_patch" => "Needs approval to edit files".to_string(),
                        other => format!("Needs approval: {other}"),
                    },
                };
                drafts.push(status(
                    &node,
                    State::InputRequired,
                    Some(truncate_chars(&summary, LABEL_MAX)),
                ));
            }
            "PreToolUse" => pre_tool_use(input, session, &node, capture, &mut drafts),
            "PostToolUse" => post_tool_use(input, session, &node, &mut drafts),
            // Nothing the graph shows.
            "PreCompact" | "PostCompact" => {}
            other => drafts.push(Draft::new(
                &node,
                Payload::Unknown(serde_json::json!({
                    "hook_event_name": other,
                    "tool_name": str_at(input, &["tool_name"]),
                })),
            )),
        }

        // Before the turn's status: any other event on the session would
        // take it back to working.
        if matches!(hook, "Stop" | "Interrupt") {
            let mut ended = unstarted_agents(session, &session_node);
            ended.extend(errored_agents_of(input, session));
            drafts.splice(0..0, ended);
        }

        Ok(Translation {
            file_key: file_key(PROVIDER, session),
            drafts,
        })
    }
}

fn pre_tool_use(
    input: &Value,
    session: &str,
    node: &str,
    capture: Capture,
    drafts: &mut Vec<Draft>,
) {
    let Some(call_id) = tool_use_id(input) else {
        return;
    };
    let tool_input = input.get("tool_input").unwrap_or(&Value::Null);
    match tool_name(input) {
        // Starting another agent from the shell: a separate session this one
        // waits for, unless it's in the background.
        "Bash" => {
            if let Some(launch) = agent_launch(input) {
                drafts.push(Draft::new(
                    node,
                    Payload::SpawnRequested(SpawnRequested {
                        call_id: call_id.to_string(),
                        kind: SpawnKind::Session,
                        agent_type: Some(launch.program),
                        purpose: None,
                        background: launch.background,
                        run: Some(launch.run),
                    }),
                ));
            }
        }
        // Returns at once, with the agent running: it's waited for, if at
        // all, with `wait_agent`. What it's asked is a prompt, so it's only
        // kept with bodies.
        "spawn_agent" => drafts.push(Draft::new(
            node,
            Payload::SpawnRequested(SpawnRequested {
                call_id: call_id.to_string(),
                kind: SpawnKind::Agent,
                agent_type: str_at(tool_input, &["agent_type"]).map(String::from),
                // v2 names the task, which is a label; v1 only has the
                // prompt, kept only with bodies.
                purpose: str_at(tool_input, &["task_name"])
                    .and_then(task_label)
                    .or_else(|| {
                        capture
                            .bodies
                            .then(|| str_at(tool_input, &["message"]))
                            .flatten()
                            .map(|s| truncate_chars(s, LABEL_MAX))
                    }),
                background: true,
                run: None,
            }),
        )),
        "request_user_input" => {
            let questions = tool_input
                .get("questions")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            let first = str_at(tool_input, &["questions", "0", "question"])
                .map(|s| truncate_chars(s, LABEL_MAX));
            let summary = match (questions, first) {
                (n, Some(q)) if n > 1 => format!("Asks {n} questions: {q}"),
                (_, Some(q)) => format!("Asks: {q}"),
                _ => "Asks you a question".to_string(),
            };
            drafts.push(status(node, State::InputRequired, Some(summary)));
        }
        // More work for an agent that's already there (it may have
        // finished): it's working again. Codex fires no hook in it for that.
        "followup_task" | "send_message" | "send_input" | "resume_agent" => {
            let target = str_at(tool_input, &["target"]).or_else(|| str_at(tool_input, &["id"]));
            if let Some(thread) = target.and_then(|t| agent_thread(input, session, t)) {
                drafts.push(status(
                    &node_id(session, Some(&thread)),
                    State::Working,
                    None,
                ));
            }
        }
        name if is_wait(name) => {
            for (i, target) in wait_targets(tool_input).enumerate() {
                drafts.push(Draft::new(
                    node,
                    Payload::WaitStarted(WaitStarted {
                        wait_id: format!("{call_id}#{i}"),
                        on: node_id(session, Some(target)),
                        reason: Some("wait_agent".into()),
                    }),
                ));
            }
        }
        _ => {}
    }
}

fn post_tool_use(input: &Value, session: &str, node: &str, drafts: &mut Vec<Draft>) {
    let tool_input = input.get("tool_input").unwrap_or(&Value::Null);
    let Some(call_id) = tool_use_id(input) else {
        return;
    };
    match tool_name(input) {
        "Bash" if agent_launch(input).is_some() => {
            // The child isn't named here; the reducer pairs it with the
            // session that linked itself to this one.
            drafts.push(Draft::new(
                node,
                Payload::SpawnReturned(SpawnReturned {
                    call_id: call_id.to_string(),
                    child: None,
                    outcome: None,
                }),
            ));
        }
        // With approvals on, one may have been asked for and given: Codex
        // has no event for that, so a command or edit finishing says so.
        "Bash" | "apply_patch" => {
            if str_at(input, &["permission_mode"]) == Some("default") {
                drafts.push(status(node, State::Working, None));
            }
        }
        // Answered: back to work.
        "request_user_input" => drafts.push(status(node, State::Working, None)),
        "spawn_agent" => {
            // Its result, as JSON text: `{"agent_id": …, "nickname": …}`.
            // v2's names only the task (`{"task_name": …}`): the request is
            // left open, for the child to be bound to it when it starts. (It
            // runs in the background, so nothing waits on it.)
            let Some(child) = json_response(input)
                .as_ref()
                .and_then(|r| str_at(r, &["agent_id"]).map(|a| node_id(session, Some(a))))
            else {
                return;
            };
            let child = Some(child);
            drafts.push(Draft::new(
                node,
                Payload::SpawnReturned(SpawnReturned {
                    call_id: call_id.to_string(),
                    child,
                    outcome: None,
                }),
            ));
        }
        "update_plan" => {
            let Some(plan) = tool_input.get("plan").and_then(Value::as_array) else {
                return;
            };
            let items = plan
                .iter()
                .enumerate()
                .filter_map(|(i, step)| {
                    Some(TaskItem {
                        id: (i + 1).to_string(),
                        text: truncate_chars(str_at(step, &["step"])?, LABEL_MAX),
                        active_text: None,
                        status: task_status(str_at(step, &["status"])?)?,
                    })
                })
                .collect();
            drafts.push(Draft::new(
                node,
                Payload::TasksUpdated(TasksUpdated { items }),
            ));
        }
        name if is_wait(name) => {
            let response = json_response(input);
            let timed_out = response
                .as_ref()
                .and_then(|r| r.get("timed_out").and_then(Value::as_bool))
                .unwrap_or(false);
            // An agent whose run broke (a model or API error, say) is
            // `errored` in Codex's own account of it: failed. One that only
            // reports something went wrong has completed.
            for (agent, error) in response.as_ref().map(errored_agents).unwrap_or_default() {
                let Some(thread) = agent_thread(input, session, &agent) else {
                    continue;
                };
                drafts.push(Draft::new(
                    node_id(session, Some(&thread)),
                    Payload::AgentFinished(AgentFinished {
                        status: FinishStatus::Failed,
                        summary: Some(truncate_chars(&error, LABEL_MAX)),
                    }),
                ));
            }
            for (i, _) in wait_targets(tool_input).enumerate() {
                drafts.push(Draft::new(
                    node,
                    Payload::WaitEnded(WaitEnded {
                        wait_id: format!("{call_id}#{i}"),
                        outcome: Some(if timed_out { "timed_out" } else { "done" }.into()),
                    }),
                ));
            }
        }
        _ => {}
    }
}

/// The agents a `wait_agent` result says errored, with Codex's error: v1's
/// `{"status": {<thread id>: {"errored": …}}}`, or v2's `{"agents":
/// [{"agent_name": <task path>, "agent_status": {"errored": …}}]}`.
fn errored_agents(response: &Value) -> Vec<(String, String)> {
    let error = |status: &Value| str_at(status, &["errored"]).map(String::from);
    let v1 = response
        .get("status")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(id, status)| Some((id.clone(), error(status)?)));
    let v2 = response
        .get("agents")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|agent| {
            let name = str_at(agent, &["agent_name"])?;
            Some((name.to_string(), error(agent.get("agent_status")?)?))
        });
    v1.chain(v2).collect()
}

/// The thread of the agent `target` names: v1 names it by its thread id;
/// v2 by its task's path (`/root/wait_60_seconds`), or relative to the
/// session's (`wait_60_seconds`), which only the agent's rollout records,
/// so the session's recent rollouts are looked through for it.
fn agent_thread(input: &Value, session: &str, target: &str) -> Option<String> {
    if !target.contains('/') && safe_id(target) && target.len() >= 32 && target.contains('-') {
        return Some(target.to_string());
    }
    let rollout = std::path::Path::new(str_at(input, &["transcript_path"])?);
    // sessions/YYYY/MM/DD/rollout-….jsonl
    let sessions = rollout.parent()?.parent()?.parent()?.parent()?;
    let wanted = |path: &str| path == target || path.ends_with(&format!("/{target}"));
    recent_rollouts(sessions, 7)
        .into_iter()
        .filter_map(|path| session_meta(&path))
        .filter(|meta| str_at(meta, &["session_id"]) == Some(session))
        .find(|meta| str_at(meta, &["agent_path"]).is_some_and(wanted))
        .and_then(|meta| {
            str_at(&meta, &["id"])
                .filter(|id| safe_id(id))
                .map(String::from)
        })
}

/// The rollouts in Codex's `sessions` folder from its last `days` days'
/// folders (`YYYY/MM/DD`), newest first.
fn recent_rollouts(sessions: &std::path::Path, days: usize) -> Vec<std::path::PathBuf> {
    let children = |dir: &std::path::Path| -> Vec<std::path::PathBuf> {
        let mut found: Vec<_> = std::fs::read_dir(dir)
            .map(|entries| entries.flatten().map(|e| e.path()).collect())
            .unwrap_or_default();
        found.sort();
        found.reverse();
        found
    };
    let day_dirs = children(sessions)
        .into_iter()
        .filter(|p| p.is_dir())
        .flat_map(|year| children(&year))
        .filter(|p| p.is_dir())
        .flat_map(|month| children(&month))
        .filter(|p| p.is_dir())
        .take(days);
    day_dirs
        .flat_map(|day| children(&day))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("rollout-") && n.ends_with(".jsonl"))
        })
        .collect()
}

/// `wait_agent` (by `tool_name`, without its namespace).
fn is_wait(name: &str) -> bool {
    name == "wait_agent"
}

/// The agents a `wait_agent` call waits for (v1 names them; v2 doesn't).
fn wait_targets(tool_input: &Value) -> impl Iterator<Item = &str> {
    tool_input
        .get("targets")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|t| safe_id(t))
}

/// A thread id fit for a node id: Codex's are UUIDs.
fn safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// A tool's response, when it's JSON (as an object, or as JSON text).
fn json_response(input: &Value) -> Option<Value> {
    match input.get("tool_response")? {
        Value::String(text) => serde_json::from_str(text).ok(),
        other @ Value::Object(_) => Some(other.clone()),
        _ => None,
    }
}

fn status(node: &str, state: State, summary: Option<String>) -> Draft {
    Draft::new(
        node,
        Payload::Status(Status {
            state,
            summary,
            title: None,
        }),
    )
}

/// The session's status, with its name if Codex has given it one.
fn titled(node: &str, session: &str, state: State) -> Draft {
    Draft::new(
        node,
        Payload::Status(Status {
            state,
            summary: None,
            title: title_of(session),
        }),
    )
}

/// The question the turn's last message ends on, if it ends on one: its
/// last line, ending in a question mark, without Markdown's emphasis.
fn asks_in_words(input: &Value) -> Option<String> {
    let message = str_at(input, &["last_assistant_message"])?;
    let line = message
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())?;
    let line = line.replace("**", "").replace('`', "");
    let line = line.trim().trim_end_matches(['*', '_']).trim_end();
    line.ends_with('?').then(|| line.to_string())
}

/// How much of the end of a session's event log to read for its last event.
const EVENTS_TAIL: u64 = 64 * 1024;

/// The question the session asked (`request_user_input`), if it's the last
/// thing recorded of the session's own node: nothing since has answered it.
fn open_question(session: &str, node: &str) -> Option<String> {
    let last = recorded(session)
        .into_iter()
        .rev()
        .find(|event| str_at(event, &["node"]) == Some(node))?;
    let summary = str_at(&last, &["data", "summary"])?;
    (str_at(&last, &["type"]) == Some("status")
        && str_at(&last, &["data", "state"]) == Some("input_required")
        && summary.starts_with("Asks"))
    .then(|| summary.to_string())
}

/// The session's recent events, as recorded (its events file's last
/// `EVENTS_TAIL` bytes), oldest first.
fn recorded(session: &str) -> Vec<Value> {
    let Some(root) = crate::paths::data_dir() else {
        return Vec::new();
    };
    let file =
        crate::paths::events_dir(&root).join(format!("{}.jsonl", file_key(PROVIDER, session)));
    read_file(&file, Some(EVENTS_TAIL))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .collect()
}

/// Agents the session asked for that never started, when its turn ends.
/// Codex runs no PostToolUse hook for a tool call that fails (a
/// `spawn_agent` with a model that doesn't exist, say), so nothing else
/// says so: a request with no reply (v1's names the agent) and no agent
/// started for its task (v2's) never started. Each becomes an agent node of
/// its own, failed, so it's seen.
fn unstarted_agents(session: &str, session_node: &str) -> Vec<Draft> {
    let events = recorded(session);
    let of_type = |kind: &'static str| {
        events
            .iter()
            .filter(move |e| str_at(e, &["type"]) == Some(kind))
    };
    let returned: std::collections::BTreeSet<&str> = of_type("spawn.returned")
        .filter_map(|e| str_at(e, &["data", "call_id"]))
        .collect();
    // The tasks of the agents that started, one per agent.
    let mut started: Vec<Option<&str>> = of_type("agent.spawned")
        .map(|e| str_at(e, &["data", "purpose"]))
        .collect();
    let mut drafts = Vec::new();
    for request in of_type("spawn.requested").filter(|e| {
        str_at(e, &["node"]) == Some(session_node) && str_at(e, &["data", "kind"]) == Some("agent")
    }) {
        let Some(call_id) = str_at(request, &["data", "call_id"]).filter(|c| safe_id(c)) else {
            continue;
        };
        if returned.contains(call_id) {
            continue;
        }
        let purpose = str_at(request, &["data", "purpose"]);
        if let Some(at) = started.iter().position(|task| *task == purpose) {
            started.remove(at);
            continue;
        }
        let agent = node_id(session, Some(call_id));
        drafts.push(Draft::new(
            session_node,
            Payload::SpawnReturned(SpawnReturned {
                call_id: call_id.to_string(),
                child: Some(agent.clone()),
                outcome: Some("failed".into()),
            }),
        ));
        drafts.push(Draft::new(
            &agent,
            Payload::AgentFinished(AgentFinished {
                status: FinishStatus::Failed,
                summary: Some("Couldn't start".into()),
            }),
        ));
    }
    drafts
}

/// Agents of the session that started and haven't finished, whose last turn
/// ended in an error (a model or API error, say), by their rollouts: Codex
/// runs no SubagentStop for a turn that errors, and v2's `wait_agent` says
/// only that the wait's over. Each is failed, with Codex's error.
fn errored_agents_of(input: &Value, session: &str) -> Vec<Draft> {
    let events = recorded(session);
    let nodes_of = |kind: &str| -> std::collections::BTreeSet<String> {
        events
            .iter()
            .filter(|e| str_at(e, &["type"]) == Some(kind))
            .filter_map(|e| str_at(e, &["node"]).map(String::from))
            .collect()
    };
    let finished = nodes_of("agent.finished");
    let open: Vec<String> = nodes_of("agent.spawned")
        .into_iter()
        .filter(|node| !finished.contains(node))
        .collect();
    if open.is_empty() {
        return Vec::new();
    }
    // sessions/YYYY/MM/DD/rollout-….jsonl
    let Some(sessions) = str_at(input, &["transcript_path"])
        .map(std::path::Path::new)
        .and_then(|p| p.parent()?.parent()?.parent()?.parent())
    else {
        return Vec::new();
    };
    let rollouts = recent_rollouts(sessions, 7);
    open.into_iter()
        .filter_map(|node| {
            let thread = node.rsplit_once('/')?.1;
            let rollout = rollouts.iter().find(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.ends_with(&format!("-{thread}.jsonl")))
            })?;
            let error = turn_error(&read_file(rollout, Some(TURN_TAIL))?)?;
            Some(Draft::new(
                &node,
                Payload::AgentFinished(AgentFinished {
                    status: FinishStatus::Failed,
                    summary: Some(truncate_chars(&error, LABEL_MAX)),
                }),
            ))
        })
        .collect()
}

/// The error a rollout's last turn ended with, if it did: its
/// `task_complete`'s `error`, whose message may itself be the API's JSON
/// reply (`{"error": {"message": …}}`).
fn turn_error(rollout: &str) -> Option<String> {
    let end = rollout
        .lines()
        .rev()
        .filter(|line| line.contains("task_complete"))
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|entry| str_at(entry, &["payload", "type"]) == Some("task_complete"))?;
    let message = str_at(&end, &["payload", "error", "message"])?;
    let inner = serde_json::from_str::<Value>(message)
        .ok()
        .and_then(|v| str_at(&v, &["error", "message"]).map(String::from));
    Some(inner.unwrap_or_else(|| message.to_string()))
}

/// How much of the end of a rollout to read for the turn that just ended.
const TURN_TAIL: u64 = 512 * 1024;

/// Whether the turn a `Stop` ends proposed a plan: in Plan mode, Codex then
/// asks whether to implement it. Its rollout records the plan as an item
/// of the turn (`{"type": "Plan", …}`); the hook's own last message has it
/// taken out. The rollout may not have the turn on disk yet when the hook
/// runs, so it's read again, for up to 3 s, till it has the plan, the
/// turn's end, or the turn's start in another mode. (The hook runs in the
/// background: Codex doesn't wait for it.)
fn plan_proposed(input: &Value) -> bool {
    let (Some(path), Some(turn)) = (
        str_at(input, &["transcript_path"]),
        str_at(input, &["turn_id"]),
    ) else {
        return false;
    };
    for _ in 0..30 {
        let Some(tail) = read_file(std::path::Path::new(path), Some(TURN_TAIL)) else {
            return false;
        };
        if plan_in(&tail, turn) {
            return true;
        }
        // Done: the turn's not in Plan mode, or it ended (its end is on
        // disk, so its plan would be too). Codex may not have written even
        // the turn's start yet: then it's read again.
        let mode = turn_entry(&tail, turn, "turn_context")
            .map(|c| str_at(&c, &["payload", "collaboration_mode", "mode"]) == Some("plan"));
        if mode == Some(false) || turn_entry(&tail, turn, "task_complete").is_some() {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    false
}

/// The rollout entry of `kind` (an entry's own type, or its payload's)
/// for turn `turn`.
fn turn_entry(rollout: &str, turn: &str, kind: &str) -> Option<Value> {
    rollout
        .lines()
        .filter(|line| line.contains(turn) && line.contains(kind))
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|entry| {
            let own = str_at(entry, &["type"]) == Some(kind);
            let payload = str_at(entry, &["payload", "type"]) == Some(kind);
            (own || payload) && str_at(entry, &["payload", "turn_id"]) == Some(turn)
        })
}

/// Whether rollout text has a plan item completed in turn `turn`.
fn plan_in(rollout: &str, turn: &str) -> bool {
    rollout
        .lines()
        .filter(|line| line.contains("\"Plan\"") && line.contains(turn))
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .any(|entry| {
            str_at(&entry, &["payload", "type"]) == Some("item_completed")
                && str_at(&entry, &["payload", "turn_id"]) == Some(turn)
                && str_at(&entry, &["payload", "item", "type"]) == Some("Plan")
        })
}

/// Codex's home: `$CODEX_HOME`, or `~/.codex`.
pub fn codex_home() -> Option<std::path::PathBuf> {
    std::env::var_os("CODEX_HOME")
        .filter(|v| !v.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| crate::paths::user_home().map(|h| h.join(".codex")))
}

/// How much of the end of `session_index.jsonl` to read for a name. It
/// gets a line per rename, of every thread, so recent ones are at its end.
const INDEX_TAIL: u64 = 256 * 1024;

/// Thread `id`'s name, from Codex's `session_index.jsonl`.
pub fn title_of(id: &str) -> Option<String> {
    let index = codex_home()?.join("session_index.jsonl");
    title_in(&read_file(&index, Some(INDEX_TAIL))?, id)
}

/// Thread `id`'s name in the text of a `session_index.jsonl`: the last line
/// for it wins, and an empty name clears it.
pub fn title_in(index: &str, id: &str) -> Option<String> {
    let entry = index
        .lines()
        .rev()
        .filter(|line| line.contains(id))
        .find_map(|line| {
            let entry = serde_json::from_str::<Value>(line).ok()?;
            (str_at(&entry, &["id"]) == Some(id)).then_some(entry)
        })?;
    let name = entry.get("thread_name")?.as_str()?.trim();
    (!name.is_empty()).then(|| truncate_chars(name, LABEL_MAX))
}

/// Longest first line of a rollout we read: it holds the thread's base
/// instructions, which run to tens of kilobytes.
const META_MAX: u64 = 1024 * 1024;

/// The `session_meta` at the start of a rollout file (its payload).
pub fn session_meta(path: &std::path::Path) -> Option<Value> {
    use std::io::{BufRead, Read};
    let file = std::fs::File::open(path).ok()?;
    let mut line = String::new();
    std::io::BufReader::new(file.take(META_MAX))
        .read_line(&mut line)
        .ok()?;
    let entry: Value = serde_json::from_str(&line).ok()?;
    (str_at(&entry, &["type"]) == Some("session_meta"))
        .then(|| entry.get("payload").cloned())
        .flatten()
}

/// The agent a `Bash` call starts, if it starts one.
fn agent_launch(input: &Value) -> Option<shell::Launch> {
    let command = str_at(input, &["tool_input", "command"])?;
    let extra = shell::extra_commands(std::env::var("AGENT_GRAPH_AGENT_COMMANDS").ok().as_deref());
    shell::agent_launch(command, &extra)
}

/// The tool a hook is for, without the namespace Codex runs into the names
/// of its multi-agent tools (`multi_agent_v1wait_agent`,
/// `collaborationspawn_agent`).
fn tool_name(input: &Value) -> &str {
    let name = str_at(input, &["tool_name"]).unwrap_or("");
    ["multi_agent_v1", "collaboration"]
        .iter()
        .find_map(|ns| {
            name.strip_prefix(ns)
                .filter(|rest| AGENT_TOOLS.contains(rest))
        })
        .unwrap_or(name)
}

/// Codex's multi-agent tools (v1's and v2's), whose names reach hooks with
/// their namespace in front.
const AGENT_TOOLS: &[&str] = &[
    "spawn_agent",
    "wait_agent",
    "send_input",
    "resume_agent",
    "close_agent",
    "send_message",
    "followup_task",
    "interrupt_agent",
    "list_agents",
];

/// A multi-agent v2 task's name as Codex's app shows it: the last part of
/// its path (`/root/wait_60_seconds`), in words: "Wait 60 seconds".
pub fn task_label(task: &str) -> Option<String> {
    let last = task.rsplit('/').find(|p| !p.is_empty())?;
    let words = last.replace(['_', '-'], " ");
    let words = words.trim();
    let mut chars = words.chars();
    let first = chars.next()?;
    Some(truncate_chars(
        &format!("{}{}", first.to_uppercase(), chars.as_str()),
        LABEL_MAX,
    ))
}

fn tool_use_id(input: &Value) -> Option<&str> {
    str_at(input, &["tool_use_id"])
}

fn task_status(s: &str) -> Option<TaskStatus> {
    match s {
        "pending" => Some(TaskStatus::Pending),
        "in_progress" => Some(TaskStatus::InProgress),
        "completed" => Some(TaskStatus::Completed),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_the_last_one_given_and_an_empty_one_clears_it() {
        let index = [
            r#"{"id":"a","thread_name":"First","updated_at":"2026-09-30T10:00:00Z"}"#,
            r#"{"id":"b","thread_name":"Other","updated_at":"2026-09-30T10:01:00Z"}"#,
            r#"{"id":"a","thread_name":"Renamed","updated_at":"2026-09-30T10:02:00Z"}"#,
        ]
        .join("\n");
        assert_eq!(title_in(&index, "a").as_deref(), Some("Renamed"));
        assert_eq!(title_in(&index, "b").as_deref(), Some("Other"));
        assert_eq!(title_in(&index, "c"), None);
        let cleared = format!(
            "{index}\n{}",
            r#"{"id":"a","thread_name":"","updated_at":"x"}"#
        );
        assert_eq!(title_in(&cleared, "a"), None);
    }

    #[test]
    fn waits_are_only_on_ids_fit_for_node_ids() {
        let input = serde_json::json!({"targets": ["01a0-b", "x y", "", 3]});
        assert_eq!(wait_targets(&input).collect::<Vec<_>>(), ["01a0-b"]);
        for (hooked, tool) in [
            ("multi_agent_v1wait_agent", "wait_agent"),
            ("collaborationspawn_agent", "spawn_agent"),
            ("spawn_agent", "spawn_agent"),
            ("collaborationwait_agent_x", "collaborationwait_agent_x"),
            ("Bash", "Bash"),
        ] {
            assert_eq!(tool_name(&serde_json::json!({ "tool_name": hooked })), tool);
        }
    }

    #[test]
    fn a_followup_finds_its_agent_by_its_tasks_path() {
        let codex = tempfile::tempdir().unwrap();
        let day = codex.path().join("sessions/2026/10/01");
        std::fs::create_dir_all(&day).unwrap();
        let meta = |id: &str, session: &str, path: Option<&str>| {
            let mut payload = serde_json::json!({"id": id, "session_id": session});
            if let Some(path) = path {
                payload["agent_path"] = path.into();
            }
            let line = serde_json::json!({"type": "session_meta", "payload": payload});
            std::fs::write(
                day.join(format!("rollout-2026-10-01T10-00-00-{id}.jsonl")),
                format!("{line}\n"),
            )
            .unwrap();
        };
        meta("root-1", "root-1", None);
        meta("child-1", "root-1", Some("/root/wait_60_seconds"));
        meta("child-2", "root-1", Some("/root/wait_120_seconds"));
        meta("other-1", "root-2", Some("/root/wait_60_seconds"));
        let input = serde_json::json!({
            "transcript_path": day.join("rollout-2026-10-01T10-00-00-root-1.jsonl"),
        });
        assert_eq!(
            agent_thread(&input, "root-1", "wait_60_seconds").as_deref(),
            Some("child-1")
        );
        assert_eq!(
            agent_thread(&input, "root-1", "/root/wait_120_seconds").as_deref(),
            Some("child-2")
        );
        assert_eq!(agent_thread(&input, "root-1", "nobody"), None);
        // v1 names the thread itself.
        let thread = "01a0f6ba-f227-72d1-896d-c966f41ac39c";
        assert_eq!(
            agent_thread(&serde_json::json!({}), "root-1", thread).as_deref(),
            Some(thread)
        );
    }

    #[test]
    fn a_plan_proposed_in_the_turn_is_found() {
        let line = |turn: &str, kind: &str| {
            serde_json::json!({
                "timestamp": "2026-10-01T00:00:00Z", "type": "event_msg",
                "payload": {"type": "item_completed", "turn_id": turn, "item": {"type": kind, "id": "i", "text": "1. Do it"}}
            })
            .to_string()
        };
        let rollout = [line("t1", "Plan"), line("t2", "AgentMessage")].join("\n");
        assert!(plan_in(&rollout, "t1"));
        assert!(!plan_in(&rollout, "t2"), "not this turn's");
        assert!(!plan_in(&rollout, "t3"));
        let context = serde_json::json!({
            "type": "turn_context",
            "payload": {"turn_id": "t1", "collaboration_mode": {"mode": "plan"}}
        })
        .to_string();
        let mode = turn_entry(&context, "t1", "turn_context").unwrap();
        assert_eq!(
            str_at(&mode, &["payload", "collaboration_mode", "mode"]),
            Some("plan")
        );
        assert!(turn_entry(&context, "t2", "turn_context").is_none());
    }

    #[test]
    fn a_tasks_name_is_said_in_words() {
        assert_eq!(
            task_label("/root/wait_60_seconds").as_deref(),
            Some("Wait 60 seconds")
        );
        assert_eq!(
            task_label("fix-the-build").as_deref(),
            Some("Fix the build")
        );
        assert_eq!(task_label("/root/").as_deref(), Some("Root"));
        assert_eq!(task_label("/"), None);
        assert_eq!(task_label("__"), None);
    }
}
