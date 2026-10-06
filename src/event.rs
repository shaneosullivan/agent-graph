//! The event envelope and the typed payload for each event type.
//!
//! `schema/event.schema.json` is the published contract for these types. Keep
//! the two in step; `tests/schema.rs` checks that what we emit validates.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const SCHEMA_VERSION: u32 = 1;

/// One line in an events file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Envelope {
    pub v: u32,
    /// ULID: sorts by time and never collides.
    pub id: String,
    /// RFC 3339 UTC timestamp with milliseconds.
    pub ts: String,
    #[serde(rename = "type")]
    pub kind: String,
    /// `<provider>:<session_id>` for a session, `<provider>:<session_id>/<agent_id>` for a subagent.
    pub node: String,
    /// Set on events that create a node, when the parent is known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
    /// Optional W3C Trace Context, e.g. `{"traceparent": "00-…"}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<Value>,
    #[serde(default)]
    pub data: Value,
}

impl Envelope {
    pub fn payload(&self) -> Payload {
        Payload::parse(&self.kind, &self.data)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Source {
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Working,
    /// Blocked on the human: a permission prompt, a question, a form.
    InputRequired,
    /// The turn is over; the session is alive and waiting for its next prompt.
    Idle,
    Completed,
    Failed,
    Canceled,
}

impl State {
    pub fn is_terminal(self) -> bool {
        matches!(self, State::Completed | State::Failed | State::Canceled)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            State::Working => "working",
            State::InputRequired => "input_required",
            State::Idle => "idle",
            State::Completed => "completed",
            State::Failed => "failed",
            State::Canceled => "canceled",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
}

impl TaskStatus {
    pub fn is_open(self) -> bool {
        !matches!(self, TaskStatus::Completed)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum FinishStatus {
    #[default]
    Completed,
    Failed,
    Canceled,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SpawnKind {
    /// A subagent inside the same provider session.
    #[default]
    Agent,
    /// A separate session, e.g. `codex exec` run from a shell tool.
    Session,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SessionStarted {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// How the session started: startup, resume, clear, compact, …
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transcript_path: Option<String>,
    /// How the envelope's `parent` was found: `env` (inherited from the
    /// parent's shell) or `run` (set by `agent-graph run`). Without a
    /// `parent`, the reducer may still link the session through `ancestors`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_method: Option<String>,
    /// The agent's process, as `<pid>@<start time>`: the start time makes it
    /// unique even after the pid is reused.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process: Option<String>,
    /// The processes above it, nearest first, in the same form. A session
    /// whose `process` appears here started this one.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ancestors: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SessionEnded {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AgentSpawned {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<bool>,
    /// The spawn request (on the agent's parent) that started it, where the
    /// provider says so (Cursor's `subagentStart`): it's paired with that
    /// one, not guessed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AgentFinished {
    pub status: FinishStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Whether it's a helper its agent ran on its own between turns
    /// (Claude Code's memory extraction, prompt suggestions, summaries),
    /// which has no type or transcript and was never said to start: one
    /// to count, not draw. Unsaid by other providers, and in logs from
    /// before it was recorded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Status {
    pub state: State,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// The session's name, as its agent shows it, when the agent has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// How the turn ended, from its last reply (a question for you), sent
    /// apart from the provider's own end of the turn. A plain end of the
    /// turn (idle) landing just after it, as Cursor's `stop` can, since its
    /// hooks run in parallel, doesn't replace it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub turn_end: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskItem {
    pub id: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_text: Option<String>,
    pub status: TaskStatus,
}

/// Replaces the node's whole task list. For providers whose todo tool sends the full list.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TasksUpdated {
    pub items: Vec<TaskItem>,
}

/// Creates or updates one task. For providers whose task tools are incremental.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskUpserted {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<TaskStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskDeleted {
    pub id: String,
}

/// The node asked for a child to be started. A foreground request means the
/// node is blocked until the matching `spawn.returned`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpawnRequested {
    pub call_id: String,
    #[serde(default)]
    pub kind: SpawnKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    #[serde(default)]
    pub background: bool,
    /// For a session: whether it's for `agent-graph run`, whose session is
    /// named for the run, not the program. Only a run's session is paired
    /// with one that is, and a run's session only with such a request. Not
    /// known in logs from before it was recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<bool>,
    /// For a session suggested to you, which the app starts if and when you
    /// accept it (Claude Code's `spawn_task`): the name it's given. Such a
    /// request is paired with the session that takes that name after it,
    /// however much later, and only with it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// The spawning call returned. `child`, when the provider reports it, is authoritative.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpawnReturned {
    pub call_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WaitStarted {
    pub wait_id: String,
    /// The node id being waited on.
    pub on: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WaitEnded {
    pub wait_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MessageSent {
    pub message_id: String,
    /// A node id, or the provider's own name for the recipient.
    pub to: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
    /// A short preview. Kept by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// The full text. Only recorded when body capture is on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

/// The `activity` tool that marks a turn in Plan mode (Cursor's), which
/// ends on a plan waiting for the human.
pub const PLAN_MODE: &str = "Plan mode";

/// The `activity` tool that marks a turn as stopped before it finished (by
/// the human, say), just before the status that ends it: a subagent it
/// started in the foreground, and that hasn't said it's finished, was
/// stopped with it, where otherwise it's taken to have finished with it.
pub const TURN_STOPPED: &str = "Turn stopped";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Activity {
    pub tool: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Until the node's next event, it may be waiting for the human to
    /// approve this (a command Cursor runs outside its sandbox, which it
    /// may ask about first: no hook says whether it has).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub may_ask: bool,
}

/// The whole state of a log's reducer at a point, so the log can carry on
/// from there without what came before (see `reducer::keyframe`): the
/// state's JSON, as text, split into parts, each a line.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Keyframe {
    pub part: usize,
    pub parts: usize,
    pub text: String,
    /// Its log's readers should start again here: an event came late, and
    /// the keyframe before hasn't it in its place (see `remote::Stream`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub restart: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Payload {
    SessionStarted(SessionStarted),
    SessionEnded(SessionEnded),
    AgentSpawned(AgentSpawned),
    AgentFinished(AgentFinished),
    Status(Status),
    TasksUpdated(TasksUpdated),
    TaskUpserted(TaskUpserted),
    TaskDeleted(TaskDeleted),
    SpawnRequested(SpawnRequested),
    SpawnReturned(SpawnReturned),
    WaitStarted(WaitStarted),
    WaitEnded(WaitEnded),
    MessageSent(MessageSent),
    Activity(Activity),
    Keyframe(Keyframe),
    /// An event type we don't know, or a known type whose data didn't parse.
    /// Consumers must ignore these rather than fail.
    Unknown(Value),
}

macro_rules! payload_types {
    ($($variant:ident => $name:literal),* $(,)?) => {
        /// Every event type name, in schema order.
        pub const EVENT_TYPES: &[&str] = &[$($name,)* "unknown"];

        impl Payload {
            pub fn type_name(&self) -> &'static str {
                match self {
                    $(Payload::$variant(_) => $name,)*
                    Payload::Unknown(_) => "unknown",
                }
            }

            pub fn to_data(&self) -> Value {
                match self {
                    $(Payload::$variant(d) => serde_json::to_value(d).unwrap_or(Value::Null),)*
                    Payload::Unknown(v) => v.clone(),
                }
            }

            pub fn parse(kind: &str, data: &Value) -> Payload {
                // Types whose fields are all optional still need an object to parse from.
                let data = if data.is_null() { &Value::Object(Default::default()) } else { data };
                match kind {
                    $($name => serde_json::from_value(data.clone())
                        .map(Payload::$variant)
                        .unwrap_or_else(|_| Payload::Unknown(data.clone())),)*
                    _ => Payload::Unknown(data.clone()),
                }
            }
        }
    };
}

payload_types! {
    SessionStarted => "session.started",
    SessionEnded => "session.ended",
    AgentSpawned => "agent.spawned",
    AgentFinished => "agent.finished",
    Status => "status",
    TasksUpdated => "tasks.updated",
    TaskUpserted => "task.upserted",
    TaskDeleted => "task.deleted",
    SpawnRequested => "spawn.requested",
    SpawnReturned => "spawn.returned",
    WaitStarted => "wait.started",
    WaitEnded => "wait.ended",
    MessageSent => "message.sent",
    Activity => "activity",
    Keyframe => "keyframe",
}

/// Shortens `s` to at most `max` characters, marking the cut with `…`.
pub fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Shortens every string inside `value` to at most `max` characters.
pub fn truncate_strings(value: &mut Value, max: usize) {
    match value {
        Value::String(s) => {
            if s.chars().count() > max {
                *s = truncate_chars(s, max);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|v| truncate_strings(v, max)),
        Value::Object(map) => map.values_mut().for_each(|v| truncate_strings(v, max)),
        _ => {}
    }
}
