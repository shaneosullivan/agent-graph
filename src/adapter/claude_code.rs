//! Claude Code hooks → Agent Graph events.
//!
//! Every hook payload carries `session_id` and `hook_event_name`. Hooks that
//! fire inside a subagent also carry `agent_id`; `session_id` stays the parent
//! session's. Tool payloads carry `tool_name`, `tool_input`, `tool_use_id`, and
//! (after the tool runs) `tool_response`.

use serde_json::Value;

use super::{Adapter, Capture, Draft, Translation, bool_at, shell, str_at};
use crate::event::{
    AgentFinished, AgentSpawned, FinishStatus, MessageSent, Payload, SessionEnded, SessionStarted,
    SpawnKind, SpawnRequested, SpawnReturned, State, Status, TaskDeleted, TaskItem, TaskStatus,
    TaskUpserted, TasksUpdated, truncate_chars,
};
use crate::paths::file_key;

pub const PROVIDER: &str = "claude-code";

/// Longest label we keep (task text, a subagent's description, a message summary).
const LABEL_MAX: usize = 200;
/// Longest body we keep when body capture is on.
const BODY_MAX: usize = 2000;

pub struct ClaudeCode;

pub fn node_id(session: &str, agent: Option<&str>) -> String {
    match agent {
        Some(agent) => format!("{PROVIDER}:{session}/{agent}"),
        None => format!("{PROVIDER}:{session}"),
    }
}

impl Adapter for ClaudeCode {
    fn provider(&self) -> &'static str {
        PROVIDER
    }

    fn adapter_id(&self) -> &'static str {
        "claude-code@1"
    }

    fn env_file_var(&self) -> Option<&'static str> {
        // Given to SessionStart hooks; its exports apply to later Bash calls.
        Some("CLAUDE_ENV_FILE")
    }

    fn translate(&self, input: &Value, capture: Capture) -> Result<Translation, String> {
        let session = str_at(input, &["session_id"]).ok_or("payload has no session_id")?;
        let session_node = node_id(session, None);
        let agent_id = str_at(input, &["agent_id"]);
        // The node the hook fired in: the subagent if there is one, else the session.
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
                    title: None,
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
            "UserPromptSubmit" => drafts.push(status(&session_node, State::Working, None)),
            "Stop" => drafts.push(status(&node, State::Idle, None)),
            "SubagentStart" => {
                let agent = agent_id.ok_or("SubagentStart has no agent_id")?;
                drafts.push(
                    Draft::new(
                        node_id(session, Some(agent)),
                        Payload::AgentSpawned(AgentSpawned {
                            agent_type: str_at(input, &["agent_type"]).map(String::from),
                            ..Default::default()
                        }),
                    )
                    .with_parent(&session_node),
                );
            }
            "SubagentStop" => {
                let agent = agent_id.ok_or("SubagentStop has no agent_id")?;
                let summary = capture
                    .bodies
                    .then(|| str_at(input, &["last_assistant_message"]))
                    .flatten()
                    .map(|s| truncate_chars(s, BODY_MAX));
                drafts.push(Draft::new(
                    node_id(session, Some(agent)),
                    Payload::AgentFinished(AgentFinished {
                        status: FinishStatus::Completed,
                        summary,
                    }),
                ));
            }
            "Notification" => match str_at(input, &["notification_type"]).unwrap_or("") {
                "permission_prompt"
                | "agent_needs_input"
                | "elicitation_dialog"
                | "elicitation_url_dialog" => {
                    drafts.push(status(&node, State::InputRequired, label(&["message"])));
                }
                "idle_prompt" => drafts.push(status(&node, State::Idle, None)),
                _ => {}
            },
            "PreToolUse" => match tool_name(input) {
                // Tools that stop and wait for the human. The session is
                // blocked on you until the tool returns.
                "AskUserQuestion" => {
                    let questions = input
                        .pointer("/tool_input/questions")
                        .and_then(Value::as_array)
                        .map_or(0, Vec::len);
                    let first = label(&["tool_input", "questions", "0", "question"]);
                    let summary = match (questions, first) {
                        (n, Some(q)) if n > 1 => format!("Asks {n} questions: {q}"),
                        (_, Some(q)) => format!("Asks: {q}"),
                        _ => "Asks you a question".to_string(),
                    };
                    drafts.push(status(&node, State::InputRequired, Some(summary)));
                }
                "ExitPlanMode" => drafts.push(status(
                    &node,
                    State::InputRequired,
                    Some("Plan ready for your review".to_string()),
                )),
                // Starting another agent from the shell: a separate session
                // this one waits for, unless it's in the background.
                "Bash" => {
                    if let (Some(call_id), Some(launch)) = (tool_use_id(input), agent_launch(input))
                    {
                        let background = launch.background
                            || bool_at(input, &["tool_input", "run_in_background"])
                                .unwrap_or(false);
                        drafts.push(Draft::new(
                            &node,
                            Payload::SpawnRequested(SpawnRequested {
                                call_id: call_id.to_string(),
                                kind: SpawnKind::Session,
                                agent_type: Some(launch.program),
                                purpose: label(&["tool_input", "description"]),
                                background,
                                run: Some(launch.run),
                            }),
                        ));
                    }
                }
                "Agent" | "Task" if tool_use_id(input).is_some() => {
                    let call_id = tool_use_id(input).unwrap_or_default();
                    drafts.push(Draft::new(
                        &node,
                        Payload::SpawnRequested(SpawnRequested {
                            call_id: call_id.to_string(),
                            kind: SpawnKind::Agent,
                            agent_type: str_at(input, &["tool_input", "subagent_type"])
                                .map(String::from),
                            purpose: label(&["tool_input", "description"]),
                            background: bool_at(input, &["tool_input", "run_in_background"])
                                .unwrap_or(false),
                            run: None,
                        }),
                    ));
                }
                _ => {}
            },
            "PostToolUse" => post_tool_use(input, session, &node, capture, &mut drafts),
            other => drafts.push(Draft::new(
                &node,
                Payload::Unknown(serde_json::json!({
                    "hook_event_name": other,
                    "tool_name": str_at(input, &["tool_name"]),
                })),
            )),
        }

        Ok(Translation {
            file_key: file_key(PROVIDER, session),
            drafts,
        })
    }
}

fn post_tool_use(
    input: &Value,
    session: &str,
    node: &str,
    capture: Capture,
    drafts: &mut Vec<Draft>,
) {
    let tool_input = input.get("tool_input").unwrap_or(&Value::Null);
    let response = input.get("tool_response").unwrap_or(&Value::Null);
    let label = |v: &Value, path: &[&str]| str_at(v, path).map(|s| truncate_chars(s, LABEL_MAX));

    match tool_name(input) {
        // Answered: back to work.
        "AskUserQuestion" | "ExitPlanMode" => {
            drafts.push(status(node, State::Working, None));
        }
        "Bash" => {
            if let (Some(call_id), Some(_)) = (tool_use_id(input), agent_launch(input)) {
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
        }
        "Agent" | "Task" => {
            let Some(call_id) = tool_use_id(input) else {
                return;
            };
            // Background agents return at once with status `async_launched`;
            // foreground ones return when they finish. Both report `agentId`.
            drafts.push(Draft::new(
                node,
                Payload::SpawnReturned(SpawnReturned {
                    call_id: call_id.to_string(),
                    child: str_at(response, &["agentId"]).map(|a| node_id(session, Some(a))),
                    outcome: str_at(response, &["status"]).map(String::from),
                }),
            ));
        }
        "TaskCreate" => {
            let Some(id) = str_at(response, &["task", "id"]) else {
                return;
            };
            drafts.push(Draft::new(
                node,
                Payload::TaskUpserted(TaskUpserted {
                    id: id.to_string(),
                    text: label(tool_input, &["subject"])
                        .or_else(|| label(response, &["task", "subject"])),
                    active_text: label(tool_input, &["activeForm"]),
                    status: Some(TaskStatus::Pending),
                }),
            ));
        }
        "TaskUpdate" => {
            let Some(id) =
                str_at(tool_input, &["taskId"]).or_else(|| str_at(response, &["taskId"]))
            else {
                return;
            };
            let status = str_at(tool_input, &["status"]);
            if status == Some("deleted") {
                drafts.push(Draft::new(
                    node,
                    Payload::TaskDeleted(TaskDeleted { id: id.to_string() }),
                ));
                return;
            }
            drafts.push(Draft::new(
                node,
                Payload::TaskUpserted(TaskUpserted {
                    id: id.to_string(),
                    text: label(tool_input, &["subject"]),
                    active_text: label(tool_input, &["activeForm"]),
                    status: status.and_then(task_status),
                }),
            ));
        }
        "TodoWrite" => {
            let Some(todos) = tool_input.get("todos").and_then(Value::as_array) else {
                return;
            };
            let items = todos
                .iter()
                .enumerate()
                .filter_map(|(i, todo)| {
                    Some(TaskItem {
                        id: (i + 1).to_string(),
                        text: label(todo, &["content"])?,
                        active_text: label(todo, &["activeForm"]),
                        status: task_status(str_at(todo, &["status"])?)?,
                    })
                })
                .collect();
            drafts.push(Draft::new(
                node,
                Payload::TasksUpdated(TasksUpdated { items }),
            ));
        }
        "SendMessage" => {
            let Some(to) =
                str_at(tool_input, &["to"]).or_else(|| str_at(tool_input, &["recipient"]))
            else {
                return;
            };
            let message_id = str_at(response, &["msg_id"]).or_else(|| tool_use_id(input));
            let Some(message_id) = message_id else { return };
            let body = capture
                .bodies
                .then(|| {
                    str_at(tool_input, &["message"]).or_else(|| str_at(tool_input, &["content"]))
                })
                .flatten()
                .map(|s| truncate_chars(s, BODY_MAX));
            drafts.push(Draft::new(
                node,
                Payload::MessageSent(MessageSent {
                    message_id: message_id.to_string(),
                    to: to.to_string(),
                    reply_to: None,
                    summary: label(tool_input, &["summary"]),
                    body,
                }),
            ));
        }
        _ => {}
    }
}

fn status(node: &str, state: State, summary: Option<String>) -> Draft {
    Draft::new(node, Payload::Status(Status { state, summary }))
}

/// The agent a Bash call starts, if it starts one.
fn agent_launch(input: &Value) -> Option<shell::Launch> {
    let command = str_at(input, &["tool_input", "command"])?;
    let extra = shell::extra_commands(std::env::var("AGENT_GRAPH_AGENT_COMMANDS").ok().as_deref());
    shell::agent_launch(command, &extra)
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
        "completed" => Some(TaskStatus::Completed),
        _ => None,
    }
}
