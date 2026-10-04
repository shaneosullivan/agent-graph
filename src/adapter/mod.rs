//! Adapters turn one provider hook payload into zero or more Agent Graph events.
//!
//! Adapters are stateless: each hook runs in its own process, often in parallel
//! with others, so anything that needs history (binding a spawn to its child,
//! merging incremental task updates) is left to the reducer.

pub mod claude_code;
pub mod codex;
pub mod cursor;
pub mod shell;

use serde_json::Value;

use crate::event::Payload;

/// What to keep from payloads beyond short labels.
#[derive(Debug, Clone, Copy, Default)]
pub struct Capture {
    /// Keep full message bodies and final agent messages. Off by default,
    /// since they can contain anything the model saw.
    pub bodies: bool,
}

/// An event before it gets an id, timestamp and source.
#[derive(Debug, Clone, PartialEq)]
pub struct Draft {
    pub node: String,
    pub parent: Option<String>,
    /// W3C Trace Context, e.g. `{"traceparent": "00-…"}`.
    pub trace: Option<Value>,
    pub payload: Payload,
}

impl Draft {
    pub fn new(node: impl Into<String>, payload: Payload) -> Self {
        Draft {
            node: node.into(),
            parent: None,
            trace: None,
            payload,
        }
    }

    pub fn with_parent(mut self, parent: impl Into<String>) -> Self {
        self.parent = Some(parent.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Translation {
    /// The events file these drafts go to (one per provider session).
    pub file_key: String,
    pub drafts: Vec<Draft>,
}

pub trait Adapter {
    /// The provider name used in node ids, e.g. `claude-code`.
    fn provider(&self) -> &'static str;
    /// Adapter name and version, recorded in each event's `source`.
    fn adapter_id(&self) -> &'static str;
    fn translate(&self, input: &Value, capture: Capture) -> Result<Translation, String>;
    /// The environment variable naming a file of `export` lines that the
    /// provider applies to its shell commands, if it has one. That's how a
    /// session passes its identity to sessions it starts (see `link`).
    fn env_file_var(&self) -> Option<&'static str> {
        None
    }
    /// Whether the provider reads a hook's output as JSON, so `emit` must
    /// always answer with some (`{}`, or the variables a starting session
    /// passes on: see `emit`). Otherwise `emit` prints nothing.
    fn replies_with_json(&self) -> bool {
        false
    }
}

/// The provider ids the adapters record (`by_name` knows each; keep them
/// in step, and add each to the reducer's `PROVIDER_PROGRAMS`).
pub const PROVIDERS: &[&str] = &["claude-code", "codex", "cursor"];

pub fn by_name(name: &str) -> Option<Box<dyn Adapter>> {
    match name {
        "claude-code" | "claude" => Some(Box::new(claude_code::ClaudeCode)),
        "codex" => Some(Box::new(codex::Codex)),
        "cursor" => Some(Box::new(cursor::Cursor)),
        _ => None,
    }
}

/// Small helpers for reading loosely-typed hook payloads.
/// Follows `path` into `value`; numeric keys index into arrays.
pub(crate) fn str_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut v = value;
    for key in path {
        v = match (v, key.parse::<usize>()) {
            (Value::Array(items), Ok(i)) => items.get(i)?,
            _ => v.get(key)?,
        };
    }
    v.as_str().filter(|s| !s.is_empty())
}

pub(crate) fn bool_at(value: &Value, path: &[&str]) -> Option<bool> {
    let mut v = value;
    for key in path {
        v = v.get(key)?;
    }
    v.as_bool()
}
