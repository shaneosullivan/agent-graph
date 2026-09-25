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
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AgentFinished {
    pub status: FinishStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Status {
    pub state: State,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Activity {
    pub tool: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
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
