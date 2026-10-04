//! Cursor hooks → Agent Graph events. See docs/cursor.md.
//!
//! Every payload carries `conversation_id` (the chat), `generation_id` (its
//! turn) and `hook_event_name`, in camelCase (`sessionStart`, `preToolUse`).
//! Tool payloads carry `tool_name`, `tool_input` and `tool_use_id`.
//! Subagents come with `subagentStart` and `subagentStop`, which name the
//! agent (`subagent_id`: the `Task` call that started it), and the chat
//! that started it (`parent_conversation_id`).
//!
//! Checked against Cursor 3.23 and its CLI (2026.10.01). Cursor's ids can
//! hold a newline (`call-…-11` then `fc_…`): node ids use the first line.
//! Hooks inside a subagent carry the subagent's own conversation and
//! `parent_tool_call_id`, but nothing naming its chat, so they're left out.
//! Transcripts are never read: only the hook payloads themselves.

use serde_json::Value;

use super::{Adapter, Capture, Draft, Translation, bool_at, shell, str_at};
use crate::event::{
    AgentFinished, AgentSpawned, FinishStatus, Payload, SessionEnded, SessionStarted, SpawnKind,
    SpawnRequested, SpawnReturned, State, Status, TaskItem, TaskStatus, TaskUpserted, TasksUpdated,
    truncate_chars,
};
use crate::paths::file_key;

pub const PROVIDER: &str = "cursor";

/// Longest label we keep (a subagent's description, a question).
const LABEL_MAX: usize = 200;
/// Longest body we keep when body capture is on.
const BODY_MAX: usize = 2000;

pub struct Cursor;

pub fn node_id(conversation: &str, agent: Option<&str>) -> String {
    match agent {
        Some(agent) => format!("{PROVIDER}:{conversation}/{agent}"),
        None => format!("{PROVIDER}:{conversation}"),
    }
}

impl Adapter for Cursor {
    fn provider(&self) -> &'static str {
        PROVIDER
    }

    fn adapter_id(&self) -> &'static str {
        "cursor@1"
    }

    fn replies_with_json(&self) -> bool {
        // Cursor reads a hook's output as JSON, and on Windows its CLI
        // takes none at all for invalid JSON, which blocks the tool.
        true
    }

    fn translate(&self, input: &Value, capture: Capture) -> Result<Translation, String> {
        let conversation = str_at(input, &["conversation_id"])
            .or_else(|| str_at(input, &["session_id"]))
            .ok_or("payload has no conversation_id")?;
        let session = node_id(conversation, None);
        let hook = str_at(input, &["hook_event_name"]).unwrap_or("");
        // Inside a subagent: its own conversation, which nothing ties to
        // the chat it's in (a hook can't remember one from another).
        if str_at(input, &["parent_tool_call_id"]).is_some() {
            return Ok(Translation {
                file_key: file_key(PROVIDER, conversation),
                drafts: Vec::new(),
            });
        }
        let label = |path: &[&str]| str_at(input, path).map(|s| truncate_chars(s, LABEL_MAX));
        let body = |path: &[&str]| {
            capture
                .bodies
                .then(|| str_at(input, path))
                .flatten()
                .map(|s| truncate_chars(s, BODY_MAX))
        };

        let mut drafts = Vec::new();
        match hook {
            "sessionStart" => drafts.push(Draft::new(
                &session,
                Payload::SessionStarted(SessionStarted {
                    cwd: str_at(input, &["workspace_roots", "0"])
                        .or_else(|| str_at(input, &["cwd"]))
                        .map(String::from),
                    source: bool_at(input, &["is_background_agent"])
                        .unwrap_or(false)
                        .then(|| "background".to_string()),
                    transcript_path: str_at(input, &["transcript_path"]).map(String::from),
                    ..Default::default()
                }),
            )),
            "sessionEnd" => {
                if str_at(input, &["reason"]) == Some("error") {
                    drafts.push(status(
                        &session,
                        State::Failed,
                        label(&["error_message"]).or_else(|| Some("Ended with an error".into())),
                    ));
                }
                drafts.push(Draft::new(
                    &session,
                    Payload::SessionEnded(SessionEnded {
                        reason: str_at(input, &["reason"]).map(String::from),
                    }),
                ));
            }
            "beforeSubmitPrompt" => drafts.push(status(&session, State::Working, None)),
            "stop" => {
                let summary = (str_at(input, &["status"]) == Some("error"))
                    .then(|| "The turn ended with an error".to_string());
                drafts.push(status(&session, State::Idle, summary));
            }
            "subagentStart" => {
                let agent = str_at(input, &["subagent_id"])
                    .map(first_line)
                    .ok_or("subagentStart has no subagent_id")?;
                // Cursor nests subagents one level at most, but the parent
                // may be named as the subagent that started this one.
                let parent = str_at(input, &["parent_conversation_id"])
                    .filter(|p| *p != conversation && first_line(p) != agent)
                    .map_or_else(
                        || session.clone(),
                        |p| node_id(conversation, Some(first_line(p))),
                    );
                drafts.push(
                    Draft::new(
                        node_id(conversation, Some(agent)),
                        Payload::AgentSpawned(AgentSpawned {
                            agent_type: str_at(input, &["subagent_type"]).map(String::from),
                            // `task` is the agent's prompt, kept only with
                            // body capture; a description isn't.
                            purpose: label(&["description"])
                                .or_else(|| body(&["task"]).map(|t| truncate_chars(&t, LABEL_MAX))),
                            call_id: str_at(input, &["tool_call_id"]).map(String::from),
                            ..Default::default()
                        }),
                    )
                    .with_parent(parent),
                );
            }
            "subagentStop" => {
                let Some(agent) = str_at(input, &["subagent_id"]).map(first_line) else {
                    return Ok(Translation {
                        file_key: file_key(PROVIDER, conversation),
                        drafts,
                    });
                };
                let status = match str_at(input, &["status"]) {
                    Some("error") => FinishStatus::Failed,
                    Some("aborted") => FinishStatus::Canceled,
                    _ => FinishStatus::Completed,
                };
                drafts.push(Draft::new(
                    node_id(conversation, Some(agent)),
                    Payload::AgentFinished(AgentFinished {
                        status,
                        summary: body(&["summary"]),
                    }),
                ));
            }
            "preToolUse" => pre_tool_use(input, &session, &mut drafts),
            "postToolUse" => post_tool_use(input, &session, &mut drafts),
            "postToolUseFailure" => {
                // A Task call that failed returns, so the session stops
                // waiting on it.
                if let (true, Some(call_id)) = (is_task(input), tool_use_id(input)) {
                    drafts.push(Draft::new(
                        &session,
                        Payload::SpawnReturned(SpawnReturned {
                            call_id: call_id.to_string(),
                            child: None,
                            outcome: Some(
                                str_at(input, &["failure_type"])
                                    .unwrap_or("error")
                                    .to_string(),
                            ),
                        }),
                    ));
                }
            }
            other => drafts.push(Draft::new(
                &session,
                Payload::Unknown(serde_json::json!({
                    "hook_event_name": other,
                    "tool_name": str_at(input, &["tool_name"]),
                })),
            )),
        }

        Ok(Translation {
            file_key: file_key(PROVIDER, conversation),
            drafts,
        })
    }
}

fn pre_tool_use(input: &Value, session: &str, drafts: &mut Vec<Draft>) {
    let label = |path: &[&str]| str_at(input, path).map(|s| truncate_chars(s, LABEL_MAX));
    let Some(call_id) = tool_use_id(input) else {
        return;
    };
    match tool_name(input) {
        "Task" => drafts.push(Draft::new(
            session,
            Payload::SpawnRequested(SpawnRequested {
                call_id: call_id.to_string(),
                kind: SpawnKind::Agent,
                agent_type: str_at(input, &["tool_input", "subagent_type"]).map(String::from),
                purpose: label(&["tool_input", "description"]),
                background: bool_at(input, &["tool_input", "run_in_background"])
                    .or_else(|| bool_at(input, &["tool_input", "is_background"]))
                    .unwrap_or(false),
                run: None,
                title: None,
            }),
        )),
        // Starting another agent from the shell.
        "Shell" => {
            if let Some(launch) = agent_launch(input) {
                drafts.push(Draft::new(
                    session,
                    Payload::SpawnRequested(SpawnRequested {
                        call_id: call_id.to_string(),
                        kind: SpawnKind::Session,
                        agent_type: Some(launch.program),
                        purpose: None,
                        background: launch.background
                            || bool_at(input, &["tool_input", "is_background"]).unwrap_or(false),
                        run: Some(launch.run),
                        title: None,
                    }),
                ));
            }
        }
        // Waiting for you. Cursor doesn't send these to hooks yet (a bug
        // its staff have confirmed): they're here for when it does.
        "AskQuestion" => {
            let first = label(&["tool_input", "questions", "0", "prompt"])
                .or_else(|| label(&["tool_input", "questions", "0", "question"]))
                .or_else(|| label(&["tool_input", "question"]));
            let summary = first.map_or_else(
                || "Asks you a question".to_string(),
                |q| format!("Asks: {q}"),
            );
            drafts.push(status(session, State::InputRequired, Some(summary)));
        }
        "CreatePlan" => drafts.push(status(
            session,
            State::InputRequired,
            Some("Plan ready for your review".to_string()),
        )),
        _ => {}
    }
}

fn post_tool_use(input: &Value, session: &str, drafts: &mut Vec<Draft>) {
    let Some(call_id) = tool_use_id(input) else {
        return;
    };
    let returned = || {
        Draft::new(
            session,
            Payload::SpawnReturned(SpawnReturned {
                call_id: call_id.to_string(),
                child: None,
                outcome: None,
            }),
        )
    };
    let tool_input = input.get("tool_input").unwrap_or(&Value::Null);
    match tool_name(input) {
        // The child was named by its own `subagentStart`, with this call.
        "Task" => drafts.push(returned()),
        "Shell" if agent_launch(input).is_some() => drafts.push(returned()),
        "AskQuestion" | "CreatePlan" => drafts.push(status(session, State::Working, None)),
        name if TODO_TOOLS.contains(&name) => {
            let Some(todos) = tool_input.get("todos").and_then(Value::as_array) else {
                return;
            };
            let label =
                |v: &Value, path: &[&str]| str_at(v, path).map(|s| truncate_chars(s, LABEL_MAX));
            // `merge` sends only the items that changed.
            if bool_at(tool_input, &["merge"]).unwrap_or(false) {
                for (i, todo) in todos.iter().enumerate() {
                    let id =
                        str_at(todo, &["id"]).map_or_else(|| (i + 1).to_string(), String::from);
                    drafts.push(Draft::new(
                        session,
                        Payload::TaskUpserted(TaskUpserted {
                            id,
                            text: label(todo, &["content"]),
                            active_text: None,
                            status: str_at(todo, &["status"]).and_then(task_status),
                        }),
                    ));
                }
                return;
            }
            let items = todos
                .iter()
                .enumerate()
                .filter_map(|(i, todo)| {
                    Some(TaskItem {
                        id: str_at(todo, &["id"]).map_or_else(|| (i + 1).to_string(), String::from),
                        text: label(todo, &["content"])?,
                        active_text: None,
                        status: task_status(str_at(todo, &["status"])?)?,
                    })
                })
                .collect();
            drafts.push(Draft::new(
                session,
                Payload::TasksUpdated(TasksUpdated { items }),
            ));
        }
        _ => {}
    }
}

/// The names Cursor's todo tool may reach hooks by (to be settled by
/// docs/cursor.md's step 1).
const TODO_TOOLS: &[&str] = &["TodoWrite", "todo_write", "UpdateTodos"];

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

/// The first line of one of Cursor's ids, some of which hold a newline
/// (`call-<uuid>-11` then `fc_<uuid>`), for a node id. It's unique in its
/// chat alone.
fn first_line(id: &str) -> &str {
    id.lines().next().unwrap_or(id).trim()
}

/// The agent a shell command starts, if it starts one.
fn agent_launch(input: &Value) -> Option<shell::Launch> {
    let command = str_at(input, &["tool_input", "command"])?;
    let extra = shell::extra_commands(std::env::var("AGENT_GRAPH_AGENT_COMMANDS").ok().as_deref());
    shell::agent_launch(command, &extra)
}

fn is_task(input: &Value) -> bool {
    tool_name(input) == "Task"
}

fn tool_name(input: &Value) -> &str {
    str_at(input, &["tool_name"]).unwrap_or("")
}

fn tool_use_id(input: &Value) -> Option<&str> {
    str_at(input, &["tool_use_id"])
}

fn task_status(s: &str) -> Option<TaskStatus> {
    match s {
        "pending" => Some(TaskStatus::Pending),
        "in_progress" => Some(TaskStatus::InProgress),
        // A cancelled item is done with, as far as the list goes.
        "completed" | "cancelled" | "canceled" => Some(TaskStatus::Completed),
        _ => None,
    }
}
