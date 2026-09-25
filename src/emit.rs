//! `agent-graph emit --provider <name>`: the command provider hooks run.
//!
//! This runs on the agent's critical path, so it follows strict rules:
//! - Always exit 0 and never write to stdout. In Claude Code, exit code 2
//!   blocks the agent's action and stdout can be added to the model's context.
//!   Errors go to `emit.log` instead.
//! - Stay fast: parse stdin, translate, one append, exit. No network.
//! - Never trust the input: unknown events are recorded, bad input is logged.

use std::ffi::OsString;
use std::io::{self, Read};
use std::path::Path;
use std::time::SystemTime;

use serde_json::Value;

use crate::adapter::{self, Capture, Draft};
use crate::event::{Envelope, SCHEMA_VERSION, Source, truncate_strings};
use crate::{paths, store};

/// Longest line we write. Small appends stay atomic, and one huge label
/// shouldn't bloat the log.
pub const MAX_LINE: usize = 4096;

/// The emit entry point. Never returns, and always exits 0.
pub fn run_and_exit(args: &[OsString]) -> ! {
    // A panic must not print to stderr or change the exit code.
    std::panic::set_hook(Box::new(|info| log_error(&format!("panic: {info}"))));
    let _ = std::panic::catch_unwind(|| {
        if let Err(e) = run(args) {
            log_error(&e);
        }
    });
    std::process::exit(0)
}

fn run(args: &[OsString]) -> Result<(), String> {
    let provider = provider_arg(args).ok_or("emit: missing --provider")?;
    let adapter = adapter::by_name(&provider)
        .ok_or_else(|| format!("emit: unknown provider {provider:?}"))?;
    let root = paths::data_dir().ok_or("emit: can't find a home directory")?;

    let mut input = String::new();
    io::stdin()
        .read_to_string(&mut input)
        .map_err(|e| format!("emit: reading stdin: {e}"))?;

    let raw = env_flag("AGENT_GRAPH_RAW");
    let capture = Capture {
        bodies: env_flag("AGENT_GRAPH_CAPTURE_BODIES"),
    };

    let parsed: Result<Value, String> =
        serde_json::from_str(&input).map_err(|e| format!("emit: stdin is not JSON: {e}"));
    let translation = parsed
        .as_ref()
        .map_err(Clone::clone)
        .and_then(|v| adapter.translate(v, capture));

    if raw {
        let key = match &translation {
            Ok(t) => t.file_key.clone(),
            Err(_) => format!("{}-unparsed", adapter.provider()),
        };
        write_raw(&root, &key, &input)?;
    }

    let translation = translation?;
    if translation.drafts.is_empty() {
        return Ok(());
    }
    let source = Source {
        provider: adapter.provider().to_string(),
        provider_version: None,
        adapter: Some(adapter.adapter_id().to_string()),
    };
    let mut out = String::new();
    for event in stamp(translation.drafts, &source, SystemTime::now()) {
        out.push_str(&to_line(&event));
        out.push('\n');
    }

    let dir = paths::events_dir(&root);
    store::ensure_dir(&dir).map_err(|e| format!("emit: creating {}: {e}", dir.display()))?;
    let file = dir.join(format!("{}.jsonl", translation.file_key));
    store::append(&file, out.as_bytes())
        .map_err(|e| format!("emit: writing {}: {e}", file.display()))
}

/// Gives each draft an id, timestamp and source. Ids from one call are
/// monotonic, so events from the same hook keep their order.
pub fn stamp(drafts: Vec<Draft>, source: &Source, now: SystemTime) -> Vec<Envelope> {
    let ts = humantime::format_rfc3339_millis(now).to_string();
    let mut ids = ulid::Generator::new();
    drafts
        .into_iter()
        .map(|draft| Envelope {
            v: SCHEMA_VERSION,
            id: ids
                .generate_from_datetime(now)
                .map(|id| id.to_string())
                .unwrap_or_else(|_| ulid::Ulid::from_datetime(now).to_string()),
            ts: ts.clone(),
            kind: draft.payload.type_name().to_string(),
            node: draft.node,
            parent: draft.parent,
            source: Some(source.clone()),
            trace: None,
            data: draft.payload.to_data(),
        })
        .collect()
}

/// Serializes an event as one line of at most `MAX_LINE` bytes, shortening
/// its strings, or dropping its data, if it's too long.
pub fn to_line(event: &Envelope) -> String {
    let line = serde_json::to_string(event).unwrap_or_default();
    if line.len() <= MAX_LINE {
        return line;
    }
    let mut event = event.clone();
    truncate_strings(&mut event.data, 200);
    let line = serde_json::to_string(&event).unwrap_or_default();
    if line.len() <= MAX_LINE {
        return line;
    }
    // Without its data the event no longer matches its type, so record it as
    // unknown rather than as a malformed event of that type.
    event.data = serde_json::json!({ "truncated": true, "type": event.kind });
    event.kind = "unknown".to_string();
    serde_json::to_string(&event).unwrap_or_default()
}

fn write_raw(root: &Path, key: &str, input: &str) -> Result<(), String> {
    let dir = paths::raw_dir(root);
    store::ensure_dir(&dir).map_err(|e| format!("emit: creating {}: {e}", dir.display()))?;
    // Re-serialize so each payload is exactly one line.
    let line = serde_json::from_str::<Value>(input)
        .map(|v| v.to_string())
        .unwrap_or_else(|_| serde_json::json!({ "unparsed": input }).to_string());
    store::append(
        &dir.join(format!("{key}.jsonl")),
        format!("{line}\n").as_bytes(),
    )
    .map_err(|e| format!("emit: writing raw payload: {e}"))
}

fn provider_arg(args: &[OsString]) -> Option<String> {
    let mut args = args.iter().map(|a| a.to_string_lossy());
    while let Some(arg) = args.next() {
        if arg == "--provider" {
            return args.next().map(|v| v.into_owned());
        }
        if let Some(value) = arg.strip_prefix("--provider=") {
            return Some(value.to_string());
        }
    }
    None
}

fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .is_ok_and(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
}

/// Best effort: appends to `emit.log`, starting it over once it passes 1 MB.
fn log_error(message: &str) {
    let Some(root) = paths::data_dir() else {
        return;
    };
    if store::ensure_dir(&root).is_err() {
        return;
    }
    let log = paths::emit_log(&root);
    if std::fs::metadata(&log).is_ok_and(|m| m.len() > 1_000_000) {
        let _ = std::fs::remove_file(&log);
    }
    let ts = humantime::format_rfc3339_millis(SystemTime::now());
    let _ = store::append(&log, format!("{ts} {message}\n").as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Payload, State, Status};

    #[test]
    fn provider_arg_forms() {
        let args = |v: &[&str]| v.iter().map(OsString::from).collect::<Vec<_>>();
        assert_eq!(
            provider_arg(&args(&["--provider", "claude-code"])).as_deref(),
            Some("claude-code")
        );
        assert_eq!(
            provider_arg(&args(&["--provider=codex"])).as_deref(),
            Some("codex")
        );
        assert_eq!(provider_arg(&args(&["--provider"])), None);
    }

    #[test]
    fn stamped_ids_keep_order_within_a_call() {
        let draft = || {
            Draft::new(
                "x:1",
                Payload::Status(Status {
                    state: State::Idle,
                    summary: None,
                }),
            )
        };
        let source = Source {
            provider: "x".into(),
            provider_version: None,
            adapter: None,
        };
        let events = stamp(vec![draft(), draft(), draft()], &source, SystemTime::now());
        assert!(events.windows(2).all(|w| w[0].id < w[1].id));
        assert!(events.iter().all(|e| e.ts == events[0].ts));
    }

    #[test]
    fn long_lines_are_shortened() {
        let summary = "x".repeat(10_000);
        let draft = Draft::new(
            "x:1",
            Payload::Status(Status {
                state: State::Idle,
                summary: Some(summary),
            }),
        );
        let source = Source {
            provider: "x".into(),
            provider_version: None,
            adapter: None,
        };
        let event = &stamp(vec![draft], &source, SystemTime::now())[0];
        let line = to_line(event);
        assert!(line.len() <= MAX_LINE);
        assert!(line.contains('…'));
    }
}
