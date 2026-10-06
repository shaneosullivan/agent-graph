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
        // Cursor runs Claude Code's hooks too, with its own payloads (its
        // chats are recorded by its own hooks: `install cursor`).
        if input.get("cursor_version").is_some() {
            return Ok(Translation {
                file_key: file_key(PROVIDER, session),
                drafts: Vec::new(),
            });
        }
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
                    // A resumed session already has its name, as may a new one
                    // the desktop app started.
                    title: session_title(
                        input,
                        read_transcript(input, Some(TITLE_TAIL)).as_deref(),
                    ),
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
            "UserPromptSubmit" => drafts.push(titled(
                &session_node,
                State::Working,
                input,
                read_transcript(input, Some(TITLE_TAIL)).as_deref(),
            )),
            "Stop" => drafts.push(turn_over(input, &node, agent_id)),
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
                // The helpers Claude Code runs on its own between turns
                // (memory extraction, prompt suggestions, summaries) have
                // no type, and keep no transcript: an agent the Agent tool
                // started has both.
                let untyped = str_at(input, &["agent_type"]).is_none_or(str::is_empty);
                let unrecorded = str_at(input, &["agent_transcript_path"])
                    .is_none_or(|p| !std::path::Path::new(p).exists());
                drafts.push(Draft::new(
                    node_id(session, Some(agent)),
                    Payload::AgentFinished(AgentFinished {
                        status: FinishStatus::Completed,
                        summary,
                        background: Some(untyped && unrecorded),
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
                "idle_prompt" => drafts.push(turn_over(input, &node, agent_id)),
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
                                title: None,
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
                            title: None,
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
        // A session suggested to you (the desktop app shows it as a task to
        // start): the app starts it if you accept, maybe much later, named
        // this. Nothing names the child; the reducer pairs it by its name.
        SPAWN_TASK => {
            let title = str_at(tool_input, &["title"])
                .map(str::trim)
                .filter(|t| !t.is_empty());
            let (Some(call_id), Some(title)) = (tool_use_id(input), title) else {
                return;
            };
            drafts.push(Draft::new(
                node,
                Payload::SpawnRequested(SpawnRequested {
                    call_id: call_id.to_string(),
                    kind: SpawnKind::Session,
                    agent_type: None,
                    purpose: label(tool_input, &["tldr"]),
                    background: true,
                    run: Some(false),
                    title: Some(truncate_chars(title, LABEL_MAX)),
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
    Draft::new(
        node,
        Payload::Status(Status {
            state,
            summary,
            title: None,
            turn_end: false,
        }),
    )
}

/// A session's status, with its name if it has one (`session_title`).
fn titled(session: &str, state: State, input: &Value, transcript: Option<&str>) -> Draft {
    Draft::new(
        session,
        Payload::Status(Status {
            state,
            summary: None,
            title: session_title(input, transcript),
            turn_end: false,
        }),
    )
}

/// A node's status when its turn ends: idle, unless it's the session and a
/// background command it started is still running. The session wakes when
/// that command finishes, so it's still at work. Subagents' commands aren't
/// in the session's transcript; their turns end idle.
fn turn_over(input: &Value, node: &str, agent: Option<&str>) -> Draft {
    if agent.is_some() {
        return status(node, State::Idle, None);
    }
    let transcript = read_transcript(input, None);
    let running = transcript
        .as_deref()
        .is_some_and(|t| background_commands_running(t) > 0);
    let state = if running { State::Working } else { State::Idle };
    titled(node, state, input, transcript.as_deref())
}

/// The desktop app's tool for suggesting a session to start (a task chip).
pub const SPAWN_TASK: &str = "mcp__ccd_session__spawn_task";

/// How much of a transcript's end to read for the session's name. Claude
/// Code writes the name again every turn, so the last one is near the end;
/// transcripts themselves run to tens of megabytes.
const TITLE_TAIL: u64 = 256 * 1024;

/// The session's transcript, or its last `tail` bytes (from a line's start).
fn read_transcript(input: &Value, tail: Option<u64>) -> Option<String> {
    read_file(
        std::path::Path::new(str_at(input, &["transcript_path"])?),
        tail,
    )
}

/// The session's name, from the end of its transcript at `path`, or the
/// name it was started with (`app_title`).
pub fn title_of(path: &std::path::Path) -> Option<String> {
    read_file(path, Some(TITLE_TAIL))
        .as_deref()
        .and_then(title_in)
        .or_else(|| app_title(path))
}

/// The session's name: the latest in its `transcript`, or the name it was
/// started with (`app_title`).
fn session_title(input: &Value, transcript: Option<&str>) -> Option<String> {
    transcript
        .and_then(title_in)
        .or_else(|| app_title(std::path::Path::new(str_at(input, &["transcript_path"])?)))
}

/// The name a session was started with, kept beside its transcript (at
/// `<session id>/custom-title.json`). The desktop app writes it as it starts
/// a session it has named, such as one from a suggested task, so it's there
/// from the session's first event; the transcript only says so once its
/// first prompt's hook has run.
fn app_title(transcript: &std::path::Path) -> Option<String> {
    let file = transcript.with_extension("").join("custom-title.json");
    let entry = serde_json::from_str::<Value>(&read_file(&file, None)?).ok()?;
    let title = str_at(&entry, &["customTitle"])?.trim();
    (!title.is_empty()).then(|| truncate_chars(title, LABEL_MAX))
}

/// A file, or its last `tail` bytes (from a line's start).
pub(crate) fn read_file(path: &std::path::Path, tail: Option<u64>) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = tail.map_or(0, |tail| len.saturating_sub(tail));
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    Some(match text.split_once('\n') {
        Some((_, rest)) if start > 0 => rest.to_string(),
        _ => text,
    })
}

/// The session's name, as Claude Code shows it: the latest `custom-title`
/// (generated from the first prompt, or set with `/rename`).
pub fn title_in(transcript: &str) -> Option<String> {
    transcript
        .lines()
        .rev()
        .filter(|line| line.contains("\"custom-title\""))
        .find_map(|line| {
            let entry = serde_json::from_str::<Value>(line).ok()?;
            let title = str_at(&entry, &["customTitle"])?.trim();
            (str_at(&entry, &["type"]) == Some("custom-title") && !title.is_empty())
                .then(|| truncate_chars(title, LABEL_MAX))
        })
}

/// How many background commands a transcript started and hasn't heard back
/// about. A start is a tool result with a `backgroundTaskId`; a finish is a
/// `<task-notification>` naming the task.
pub fn background_commands_running(transcript: &str) -> usize {
    let mut running = std::collections::BTreeSet::new();
    let mut done = std::collections::BTreeSet::new();
    for line in transcript.lines() {
        if line.contains("\"backgroundTaskId\"") {
            if let Some(id) = serde_json::from_str::<Value>(line)
                .ok()
                .as_ref()
                .and_then(|entry| str_at(entry, &["toolUseResult", "backgroundTaskId"]))
            {
                running.insert(id.to_string());
            }
        }
        let mut rest = line;
        while let Some(start) = rest.find("<task-id>") {
            rest = &rest[start + "<task-id>".len()..];
            if let Some(end) = rest.find("</task-id>") {
                done.insert(rest[..end].to_string());
            }
        }
    }
    running.difference(&done).count()
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
