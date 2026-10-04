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
    Activity, AgentFinished, AgentSpawned, FinishStatus, PLAN_MODE, Payload, SessionEnded,
    SessionStarted, SpawnKind, SpawnRequested, SpawnReturned, State, Status, TURN_STOPPED,
    TaskItem, TaskStatus, TaskUpserted, TasksUpdated, truncate_chars,
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

    fn needs_start(&self) -> bool {
        true
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
        let hook = str_at(input, &["hook_event_name"]).unwrap_or("");
        // A CLI subagent's own conversation: its hooks are the subagent's,
        // in its chat (see `cli_subagent_here`).
        let routed = cli_subagent_here(conversation, hook);
        let (session, key) = match &routed {
            Some(r) => (r.node.clone(), file_key(PROVIDER, &r.root)),
            None => (
                node_id(conversation, None),
                file_key(PROVIDER, conversation),
            ),
        };
        // Inside a subagent: its own conversation, which nothing ties to
        // the chat it's in (a hook can't remember one from another).
        if str_at(input, &["parent_tool_call_id"]).is_some() {
            return Ok(Translation {
                file_key: key.clone(),
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
        // Each of its tool calls says what it is, and what started it: the
        // CLI says nothing of a subagent's start or end.
        if let (Some(r), "preToolUse") = (&routed, hook) {
            drafts.push(
                Draft::new(
                    &r.node,
                    Payload::AgentSpawned(AgentSpawned {
                        agent_type: r.agent_type.clone(),
                        call_id: Some(r.call_id.clone()),
                        ..Default::default()
                    }),
                )
                .with_parent(&r.parent),
            );
        }
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
                    // A resumed chat has its name already.
                    title: title_of(conversation),
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
                // A print run (`agent -p`) has no `stop`: its end is its
                // turn's too, so its tasks are recorded, and its subagents
                // finished with it, before it ends.
                if str_at(input, &["reason"]) == Some("completed") {
                    drafts.extend(turn_over(&session, conversation, false));
                }
                drafts.push(Draft::new(
                    &session,
                    Payload::SessionEnded(SessionEnded {
                        reason: str_at(input, &["reason"]).map(String::from),
                    }),
                ));
            }
            "beforeSubmitPrompt" => {
                drafts.push(titled(&session, State::Working, conversation));
                // A turn in Plan mode ends on a plan waiting for you.
                if str_at(input, &["composer_mode"]) == Some("plan") {
                    drafts.push(Draft::new(
                        &session,
                        Payload::Activity(Activity {
                            tool: PLAN_MODE.to_string(),
                            label: None,
                            may_ask: false,
                        }),
                    ));
                }
            }
            // (Its `status` says how: Cursor's CLI calls a turn stopped with
            // Esc an error, so it isn't shown as one.)
            // The chat's name, which Cursor gives it during its first turn,
            // and an app chat's todo list as the turn left it (no hook
            // sends it: Cursor saves it, as it goes, in its database).
            // Stopped before it finished (Esc, or a message sent mid-turn):
            // its subagents still at work were stopped too.
            "stop" => drafts.extend(turn_over(
                &session,
                conversation,
                str_at(input, &["status"]) == Some("aborted"),
            )),
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
                        file_key: key.clone(),
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
            // A command outside Cursor's sandbox, which Cursor may ask you
            // to approve first (one in the sandbox never asks). No hook says
            // whether it's asking: `beforeShellExecution` comes before it
            // asks, and `afterShellExecution` once the command has run.
            "beforeShellExecution" if may_ask(input) => match cli_asks(input) {
                Some(true) => drafts.push(status(
                    &session,
                    State::InputRequired,
                    Some("Waiting for your approval to run a command".to_string()),
                )),
                Some(false) => {}
                None => drafts.push(command(&session, input, true)),
            },
            // It ran (approved, or never asked about): that command, and only
            // that one (Cursor may have others queued, still to ask about), is
            // done with.
            "afterShellExecution" if may_ask(input) && cli_asked(input) != Some(false) => {
                drafts.push(command(&session, input, false));
            }
            // In the sandbox, or run without asking: nothing to record.
            "beforeShellExecution" | "afterShellExecution" => {}
            // The turn's last reply, which comes with `stop`, before or after
            // it: one ending on a question is the turn ending on it. The text
            // itself is kept only with body capture.
            "afterAgentResponse" => {
                if let Some(text) = str_at(input, &["text"]).filter(|t| ends_on_question(t)) {
                    let summary = match capture.bodies {
                        true => format!("Asks: {}", truncate_chars(last_line(text), LABEL_MAX)),
                        false => "Asks you a question".to_string(),
                    };
                    drafts.push(Draft::new(
                        &session,
                        Payload::Status(Status {
                            state: State::InputRequired,
                            summary: Some(summary),
                            title: None,
                            turn_end: true,
                        }),
                    ));
                }
            }
            "preToolUse" => pre_tool_use(input, &session, &mut drafts),
            "postToolUse" => post_tool_use(input, &session, &mut drafts),
            // A command that failed, or that you didn't allow
            // (`permission_denied`): it isn't waiting on you.
            "postToolUseFailure" if tool_name(input) == "Shell" => {
                drafts.push(command(&session, input, false));
            }
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
            file_key: key.clone(),
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
            turn_end: false,
        }),
    )
}

/// The first line of one of Cursor's ids, some of which hold a newline
/// (`call-<uuid>-11` then `fc_<uuid>`), for a node id. It's unique in its
/// chat alone.
fn first_line(id: &str) -> &str {
    id.lines().next().unwrap_or(id).trim()
}

/// A turn's end: the chat idle, with its name, in Plan mode or not, and its
/// todo list as the turn left it (no hook sends it: Cursor keeps it, as it
/// goes, in its databases: see `cli_chat`, `app_chat`).
fn turn_over(session: &str, conversation: &str, stopped: bool) -> Vec<Draft> {
    let mut drafts = Vec::new();
    if stopped {
        drafts.push(marker(session, TURN_STOPPED));
    }
    let cli = cli_chat_here(conversation);
    let plan = cli.as_ref().and_then(|c| c.mode.as_deref()) == Some("plan");
    // First, so it isn't taken for activity after the turn's end.
    let todos = match cli {
        Some(cli) => Some(cli.todos),
        None => app_chat_here(conversation).map(|c| c.todos),
    };
    if let Some(items) = todos.filter(|t| !t.is_empty()) {
        drafts.push(Draft::new(
            session,
            Payload::TasksUpdated(TasksUpdated { items }),
        ));
    }
    // The CLI's hooks don't say which mode a turn was in.
    if plan {
        drafts.push(marker(session, PLAN_MODE));
    }
    drafts.push(titled(session, State::Idle, conversation));
    drafts
}

/// An `activity` that only marks something (`PLAN_MODE`, `TURN_STOPPED`).
fn marker(session: &str, tool: &str) -> Draft {
    Draft::new(
        session,
        Payload::Activity(Activity {
            tool: tool.to_string(),
            label: None,
            may_ask: false,
        }),
    )
}

/// A CLI subagent's conversation, tied to what started it.
struct Routed {
    /// The chat at the top, whose events file it's in.
    root: String,
    /// The subagent's node: its chat's, and its `Task` call's.
    node: String,
    /// The node that started it: the chat, or another subagent.
    parent: String,
    call_id: String,
    agent_type: Option<String>,
}

/// The CLI subagent whose conversation `conversation` is, from its own
/// record in the CLI's database (`subagentInfo`: its chat, and the `Task`
/// call that started it), which its hooks don't say.
#[cfg(feature = "cli")]
fn cli_subagent_here(conversation: &str, hook: &str) -> Option<Routed> {
    std::env::var_os("CURSOR_INVOKED_AS")?;
    if matches!(hook, "sessionStart" | "beforeSubmitPrompt") {
        return None;
    }
    let chats = crate::paths::user_home()?.join(".cursor").join("chats");
    let info = cli_chat(&chats, conversation)?.subagent?;
    // Started by another subagent: that one's node.
    let parent = if info.parent == info.root {
        node_id(&info.root, None)
    } else {
        cli_chat(&chats, &info.parent)
            .and_then(|c| c.subagent)
            .map_or_else(
                || node_id(&info.root, None),
                |p| node_id(&info.root, Some(first_line(&p.call_id))),
            )
    };
    Some(Routed {
        node: node_id(&info.root, Some(first_line(&info.call_id))),
        root: info.root,
        parent,
        call_id: info.call_id,
        agent_type: info.agent_type,
    })
}

#[cfg(not(feature = "cli"))]
fn cli_subagent_here(_conversation: &str, _hook: &str) -> Option<Routed> {
    None
}

/// What the CLI keeps of `conversation`, when this hook is the CLI's.
#[cfg(feature = "cli")]
fn cli_chat_here(conversation: &str) -> Option<CliChat> {
    std::env::var_os("CURSOR_INVOKED_AS")?;
    cli_chat(
        &crate::paths::user_home()?.join(".cursor").join("chats"),
        conversation,
    )
}

#[cfg(not(feature = "cli"))]
fn cli_chat_here(_conversation: &str) -> Option<CliChat> {
    None
}

/// A chat's status, with its name if it has one.
fn titled(session: &str, state: State, conversation: &str) -> Draft {
    Draft::new(
        session,
        Payload::Status(Status {
            state,
            summary: None,
            title: title_of(conversation),
            turn_end: false,
        }),
    )
}

/// A chat's name, as Cursor shows it: a CLI chat's (`cli_title`), or an
/// app chat's (`app_titles`).
#[cfg(feature = "cli")]
pub fn title_of(conversation: &str) -> Option<String> {
    let home = crate::paths::user_home()?;
    if std::env::var_os("CURSOR_INVOKED_AS").is_some() {
        return cli_title(&home.join(".cursor").join("chats"), conversation);
    }
    app_titles(&app_store(&home)?, Some(conversation))?.remove(conversation)
}

/// (The site's WebAssembly reads no hooks.)
#[cfg(not(feature = "cli"))]
pub fn title_of(_conversation: &str) -> Option<String> {
    None
}

/// What Cursor's CLI keeps of a chat in its own database
/// (`~/.cursor/chats/<folder's hash>/<chat id>/store.db`, SQLite): its
/// metadata (a JSON object, hex-encoded, in its `meta` table), and its todo
/// list (records in `blobs`, which the root names: see `cli_todos`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CliChat {
    /// `plan` in Plan mode, else `default`.
    pub mode: Option<String>,
    /// Run Everything (`--force`, or chosen in the session).
    pub run_everything: bool,
    /// `auto-review`, or `unrestricted` with Run Everything; not there for
    /// its settings' own (allowlist).
    pub approval_mode: Option<String>,
    /// For a subagent's own conversation: what started it.
    pub subagent: Option<CliSubagent>,
    pub todos: Vec<TaskItem>,
}

/// Where a CLI subagent's conversation came from (its `subagentInfo`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CliSubagent {
    /// The conversation that started it: the chat, or a subagent of it.
    pub parent: String,
    /// The chat at the top.
    pub root: String,
    /// The `Task` call that started it.
    pub call_id: String,
    pub agent_type: Option<String>,
}

/// The folder of CLI chat `conversation`, in `chats` (Cursor's
/// `~/.cursor/chats`), under a folder for its working folder.
pub fn cli_chat_dir(chats: &std::path::Path, conversation: &str) -> Option<std::path::PathBuf> {
    if !is_id(conversation) {
        return None;
    }
    std::fs::read_dir(chats)
        .ok()?
        .flatten()
        .map(|dir| dir.path().join(conversation))
        .find(|dir| dir.is_dir())
}

/// What the CLI keeps of chat `conversation` (`CliChat`), from its database
/// in `chats`. Opened read-only; the conversation itself isn't read.
#[cfg(feature = "cli")]
pub fn cli_chat(chats: &std::path::Path, conversation: &str) -> Option<CliChat> {
    use rusqlite::{Connection, OpenFlags, OptionalExtension};
    let store = cli_chat_dir(chats, conversation)?.join("store.db");
    if !store.exists() {
        return None;
    }
    let db = Connection::open_with_flags(
        &store,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    db.busy_timeout(std::time::Duration::from_millis(200))
        .ok()?;
    let meta: String = db
        .query_row("select value from meta where key = '0'", [], |row| {
            row.get(0)
        })
        .optional()
        .ok()??;
    let meta = hex_decode(&meta)?;
    let meta: Value = serde_json::from_slice(&meta).ok()?;
    let blob = |id: &[u8]| -> Option<Vec<u8>> {
        db.query_row(
            "select data from blobs where id = ?1",
            [hex_encode(id)],
            |row| row.get(0),
        )
        .optional()
        .ok()?
    };
    let root = str_at(&meta, &["latestRootBlobId"])
        .and_then(hex_decode)
        .and_then(|id| blob(&id));
    let todos = root.map(|root| cli_todos(&root, &blob)).unwrap_or_default();
    let info = meta.get("subagentInfo");
    let subagent = info.and_then(|info| {
        Some(CliSubagent {
            parent: str_at(info, &["parentAgentId"])?.to_string(),
            root: str_at(info, &["rootParentAgentId"])
                .or_else(|| str_at(info, &["parentAgentId"]))?
                .to_string(),
            call_id: str_at(info, &["toolCallId"])?.to_string(),
            agent_type: str_at(info, &["typeName"]).map(String::from),
        })
    });
    Some(CliChat {
        mode: str_at(&meta, &["mode"]).map(String::from),
        run_everything: bool_at(&meta, &["isRunEverything"]).unwrap_or(false),
        approval_mode: str_at(&meta, &["approvalMode"]).map(String::from),
        subagent,
        todos,
    })
}

/// A CLI chat's todo list, from its root record (a protobuf message, in
/// `blobs`): field 3 names, by their SHA-256, one record per item, each
/// with its id (field 1), text (2) and status (3: 1 pending, 2 in
/// progress, 3 completed, 4 cancelled, the order of `TodoWrite`'s own).
pub fn cli_todos(root: &[u8], blob: &dyn Fn(&[u8]) -> Option<Vec<u8>>) -> Vec<TaskItem> {
    protobuf_fields(root)
        .into_iter()
        .filter(|(field, _)| *field == 3)
        .filter_map(|(_, value)| {
            let ProtoValue::Bytes(id) = value else {
                return None;
            };
            let item = blob(id)?;
            let (mut todo_id, mut text, mut status) = (None, None, None);
            for (field, value) in protobuf_fields(&item) {
                match (field, value) {
                    (1, ProtoValue::Bytes(b)) => {
                        todo_id = std::str::from_utf8(b).ok().map(String::from)
                    }
                    (2, ProtoValue::Bytes(b)) => {
                        text = std::str::from_utf8(b)
                            .ok()
                            .map(|t| truncate_chars(t, LABEL_MAX))
                    }
                    (3, ProtoValue::Int(n)) => {
                        status = match n {
                            1 => Some(TaskStatus::Pending),
                            2 => Some(TaskStatus::InProgress),
                            3 | 4 => Some(TaskStatus::Completed),
                            _ => None,
                        }
                    }
                    _ => {}
                }
            }
            Some(TaskItem {
                id: todo_id?,
                text: text?,
                active_text: None,
                status: status?,
            })
        })
        .collect()
}

/// One field's value in a protobuf message: enough of the format to read
/// the CLI's todo records.
enum ProtoValue<'a> {
    Int(u64),
    Bytes(&'a [u8]),
}

/// A protobuf message's fields, in order, as far as it can be read.
fn protobuf_fields(message: &[u8]) -> Vec<(u64, ProtoValue<'_>)> {
    fn varint(bytes: &[u8], at: &mut usize) -> Option<u64> {
        let mut value = 0u64;
        for shift in (0..64).step_by(7) {
            let byte = *bytes.get(*at)?;
            *at += 1;
            value |= u64::from(byte & 0x7f) << shift;
            if byte < 0x80 {
                return Some(value);
            }
        }
        None
    }
    let mut out = Vec::new();
    let mut at = 0;
    while at < message.len() {
        let Some(key) = varint(message, &mut at) else {
            break;
        };
        let value = match key & 7 {
            0 => match varint(message, &mut at) {
                Some(n) => ProtoValue::Int(n),
                None => break,
            },
            2 => {
                let Some(len) = varint(message, &mut at) else {
                    break;
                };
                let Some(bytes) = usize::try_from(len)
                    .ok()
                    .and_then(|len| message.get(at..at.checked_add(len)?))
                else {
                    break;
                };
                at += bytes.len();
                ProtoValue::Bytes(bytes)
            }
            1 => {
                at += 8;
                continue;
            }
            5 => {
                at += 4;
                continue;
            }
            _ => break,
        };
        out.push((key >> 3, value));
    }
    out
}

fn hex_decode(hex: &str) -> Option<Vec<u8>> {
    let hex = hex.trim();
    (hex.len() % 2 == 0)
        .then(|| {
            (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok())
                .collect()
        })
        .flatten()
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A CLI chat's name: the `title` in its `meta.json`, in `chats` (Cursor's
/// `~/.cursor/chats`), under a folder for its working folder. A print run
/// (`agent -p`) has none.
pub fn cli_title(chats: &std::path::Path, conversation: &str) -> Option<String> {
    let meta =
        std::fs::read_to_string(cli_chat_dir(chats, conversation)?.join("meta.json")).ok()?;
    let meta = serde_json::from_str::<Value>(&meta).ok()?;
    let title = str_at(&meta, &["title"])?.trim();
    (!title.is_empty()).then(|| truncate_chars(title, LABEL_MAX))
}

/// Where Cursor's app keeps its chats' names (among much else): its global
/// `state.vscdb`, a SQLite database.
pub fn app_store(home: &std::path::Path) -> Option<std::path::PathBuf> {
    let user = if cfg!(target_os = "macos") {
        home.join("Library/Application Support/Cursor/User")
    } else if cfg!(windows) {
        std::path::PathBuf::from(std::env::var_os("APPDATA")?).join("Cursor\\User")
    } else {
        home.join(".config/Cursor/User")
    };
    Some(user.join("globalStorage").join("state.vscdb"))
}

/// App chats' names, from Cursor's database at `store` (its
/// `composerHeaders` table): `only`'s, or every chat's. Opened read-only,
/// so Cursor writing it meanwhile is no matter; if it's busy, nothing is
/// read this time.
#[cfg(feature = "cli")]
pub fn app_titles(
    store: &std::path::Path,
    only: Option<&str>,
) -> Option<std::collections::BTreeMap<String, String>> {
    use rusqlite::{Connection, OpenFlags};
    if !store.exists() || only.is_some_and(|id| !is_id(id)) {
        return None;
    }
    let db = Connection::open_with_flags(
        store,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    db.busy_timeout(std::time::Duration::from_millis(200))
        .ok()?;
    let query = "select composerId, json_extract(value, '$.name') from composerHeaders \
                 where ?1 is null or composerId = ?1";
    let mut statement = db.prepare(query).ok()?;
    let rows = statement
        .query_map([only], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .ok()?;
    Some(
        rows.flatten()
            .filter_map(|(id, name)| {
                let name = name?;
                let name = name.trim();
                (!name.is_empty()).then(|| (id, truncate_chars(name, LABEL_MAX)))
            })
            .collect(),
    )
}

/// What Cursor's app has saved of a chat beyond what its hooks say, in its
/// database: whether a plan is waiting for you to build it, and its todo
/// list. Cursor saves these when it gets round to it: the todo list as it
/// changes, but a plan's being ready only just after the turn's `stop`
/// hook, so only the viewer, reading it again later, can use that.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AppChat {
    pub plan_pending: bool,
    pub todos: Vec<TaskItem>,
}

/// An app chat's saved state (`AppChat`), from Cursor's database at
/// `store`. `None` if it isn't there (a CLI chat's isn't), or can't be read.
#[cfg(feature = "cli")]
pub fn app_chat(store: &std::path::Path, conversation: &str) -> Option<AppChat> {
    use rusqlite::{Connection, OpenFlags, OptionalExtension};
    if !store.exists() || !is_id(conversation) {
        return None;
    }
    let db = Connection::open_with_flags(
        store,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    db.busy_timeout(std::time::Duration::from_millis(200))
        .ok()?;
    let (plan, todos) = db
        .query_row(
            "select json_extract(h.value, '$.hasPendingPlan'), json_extract(d.value, '$.todos') \
             from composerHeaders h left join cursorDiskKV d \
             on d.key = 'composerData:' || h.composerId where h.composerId = ?1",
            [conversation],
            |row| {
                Ok((
                    row.get::<_, Option<i64>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                ))
            },
        )
        .optional()
        .ok()??;
    let todos = todos
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .map(|t| todos_in(&t))
        .unwrap_or_default();
    Some(AppChat {
        plan_pending: plan == Some(1),
        todos,
    })
}

/// The app chat `conversation`'s saved state, when this hook is an app's
/// (the CLI's chats aren't in that database).
#[cfg(feature = "cli")]
fn app_chat_here(conversation: &str) -> Option<AppChat> {
    if std::env::var_os("CURSOR_INVOKED_AS").is_some() {
        return None;
    }
    app_chat(&app_store(&crate::paths::user_home()?)?, conversation)
}

#[cfg(not(feature = "cli"))]
fn app_chat_here(_conversation: &str) -> Option<AppChat> {
    None
}

/// A todo list as Cursor saves it: `[{"id", "content", "status"}]`, with
/// status pending, in_progress, completed or cancelled.
pub fn todos_in(todos: &Value) -> Vec<TaskItem> {
    let label = |v: &Value, path: &[&str]| str_at(v, path).map(|s| truncate_chars(s, LABEL_MAX));
    todos
        .as_array()
        .map(|items| {
            items
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
                .collect()
        })
        .unwrap_or_default()
}

/// One of Cursor's chat ids (a UUID), fit to name a file or go in a query.
fn is_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

/// A command Cursor may be asking you about (`may_ask`), or done with: an
/// `activity` naming it by a key from its turn and its text, the same in
/// each of its hooks, so each one queued is tracked on its own. The text
/// itself isn't kept.
fn command(session: &str, input: &Value, may_ask: bool) -> Draft {
    let text = str_at(input, &["command"])
        .or_else(|| str_at(input, &["tool_input", "command"]))
        .unwrap_or("");
    let turn = str_at(input, &["generation_id"]).unwrap_or("");
    Draft::new(
        session,
        Payload::Activity(Activity {
            tool: "Shell".to_string(),
            label: Some(format!("command {:016x}", fnv(&format!("{turn}\n{text}")))),
            may_ask,
        }),
    )
}

/// FNV-1a, 64 bits: a short, stable key.
fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf29ce484222325, |hash, b| {
        (hash ^ u64::from(b)).wrapping_mul(0x100000001b3)
    })
}

/// Whether a command (`beforeShellExecution`, `afterShellExecution`) is
/// outside Cursor's sandbox, so Cursor may ask about it.
fn may_ask(input: &Value) -> bool {
    bool_at(input, &["sandbox"]) != Some(true)
}

/// Whether Cursor's CLI will ask you to approve a command, as far as its
/// settings say for sure: in its Allowlist mode, a command not on the
/// allowlist. `None` in the app (whose settings aren't in a file of
/// Cursor's documented), and wherever its settings don't settle it.
fn cli_asks(input: &Value) -> Option<bool> {
    cli_decides(input, true)
}

/// `cli_asks`, as of before the command ran, for its end: Run Everything
/// chosen in the session (which the chat's record says at once) may have
/// been chosen in answer to this very command.
fn cli_asked(input: &Value) -> Option<bool> {
    cli_decides(input, false)
}

fn cli_decides(input: &Value, chat_too: bool) -> Option<bool> {
    // Only the CLI's hooks have this.
    std::env::var_os("CURSOR_INVOKED_AS")?;
    // How this run of the CLI was started can override its settings, and
    // so can the chat's own (Run Everything, chosen in the session).
    match this_run_mode()? {
        RunMode::Settings => {}
        RunMode::NeverAsks => return Some(false),
        RunMode::Unknown => return None,
    }
    let conversation = str_at(input, &["conversation_id"]).unwrap_or("");
    let chat = cli_chat_here(conversation).map(|c| match &c.subagent {
        // A subagent's is its chat's.
        Some(s) => cli_chat_here(&s.root).unwrap_or(c),
        None => c,
    });
    if let Some(chat) = chat.filter(|_| chat_too) {
        if chat.run_everything {
            return Some(false);
        }
        if chat.approval_mode.as_deref() == Some("auto-review") {
            return None;
        }
    }
    let home = crate::paths::user_home()?;
    let project = str_at(input, &["workspace_roots", "0"]).map(std::path::PathBuf::from);
    let read = |path: std::path::PathBuf| {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
    };
    let global = read(home.join(".cursor").join("cli-config.json"))?;
    let local = project.and_then(|p| read(p.join(".cursor").join("cli.json")));
    if str_at(&global, &["approvalMode"]) != Some("allowlist") {
        return None;
    }
    let rules = |key: &str| -> Vec<String> {
        [Some(&global), local.as_ref()]
            .into_iter()
            .flatten()
            .filter_map(|c| {
                c.pointer(&format!("/permissions/{key}"))?
                    .as_array()
                    .cloned()
            })
            .flatten()
            .filter_map(|r| r.as_str().map(String::from))
            .collect()
    };
    asks(
        str_at(input, &["command"])?,
        &rules("allow"),
        &rules("deny"),
    )
}

/// What a run of the CLI's command line says about asking.
#[derive(Debug, PartialEq, Eq)]
pub enum RunMode {
    /// Nothing: its settings decide.
    Settings,
    /// Run Everything (`--force`, `--yolo`), or a print run (`-p`), which
    /// refuses what it would otherwise ask about.
    NeverAsks,
    /// Auto-review (`--auto-review`): a classifier decides.
    Unknown,
}

/// What the command line of the CLI running this hook says (`None` where
/// it can't be read, as in the site's WebAssembly, which reads no hooks).
#[cfg(feature = "cli")]
fn this_run_mode() -> Option<RunMode> {
    let (cli, _) = crate::process::agent_of_this_hook()?;
    Some(run_mode(&crate::process::command_line(cli.pid)?))
}

#[cfg(not(feature = "cli"))]
fn this_run_mode() -> Option<RunMode> {
    None
}

/// What the CLI's command line (`agent …`, as `ps` shows it) says about
/// whether it asks before running a command.
pub fn run_mode(command_line: &str) -> RunMode {
    let words: Vec<&str> = command_line.split_whitespace().collect();
    let has = |flags: &[&str]| words.iter().any(|w| flags.contains(w));
    if has(&["--force", "-f", "--yolo", "-p", "--print"]) {
        RunMode::NeverAsks
    } else if has(&["--auto-review"]) {
        RunMode::Unknown
    } else {
        RunMode::Settings
    }
}

/// Whether the CLI asks before running `command`, given its allow and deny
/// rules (`Shell(git)`, `Shell(curl:*)`): it does unless a rule names the
/// command's first word (a denied command is refused, not asked about).
/// `None` for a command that chains others (`&&`, `|`, `;`), which Cursor's
/// docs don't say how it matches.
pub fn asks(command: &str, allow: &[String], deny: &[String]) -> Option<bool> {
    let command = command.trim();
    if command.is_empty()
        || ["&&", "||", "|", ";", "`", "$(", "\n", "&"]
            .iter()
            .any(|op| command.contains(op))
    {
        return None;
    }
    let (word, args) = command
        .split_once(char::is_whitespace)
        .map_or((command, ""), |(w, a)| (w, a.trim()));
    let matches = |rule: &String| {
        let Some(pattern) = rule
            .strip_prefix("Shell(")
            .and_then(|r| r.strip_suffix(')'))
        else {
            return false;
        };
        match pattern.split_once(':') {
            Some((base, wanted)) => glob(base, word) && glob(wanted, args),
            None => glob(pattern, word),
        }
    };
    Some(!deny.iter().any(matches) && !allow.iter().any(matches))
}

/// Whether `text` matches `pattern`, where `*` is any run of characters.
fn glob(pattern: &str, text: &str) -> bool {
    let mut parts = pattern.split('*');
    let first = parts.next().unwrap_or("");
    let Some(mut rest) = text.strip_prefix(first) else {
        return false;
    };
    let parts: Vec<&str> = parts.collect();
    let Some((last, middle)) = parts.split_last() else {
        return rest.is_empty();
    };
    for part in middle {
        match rest.find(part) {
            Some(i) => rest = &rest[i + part.len()..],
            None => return false,
        }
    }
    rest.ends_with(last)
}

/// Whether a reply ends on a question, past any closing formatting
/// (`**…?**`, quotes, a bracket).
fn ends_on_question(text: &str) -> bool {
    text.trim_end()
        .trim_end_matches(|c: char| "*_`\"')]”’".contains(c) || c.is_whitespace())
        .ends_with(['?', '？'])
}

/// The last line of a reply with anything on it.
fn last_line(text: &str) -> &str {
    text.lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
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
