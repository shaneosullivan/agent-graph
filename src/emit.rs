//! `agent-graph emit --provider <name>`: the command provider hooks run.
//!
//! This runs on the agent's critical path, so it follows strict rules:
//! - Always exit 0 and never write to stdout. In Claude Code, exit code 2
//!   blocks the agent's action and stdout can be added to the model's context.
//!   Errors go to `emit.log` instead. The exception is a provider that reads
//!   a hook's output as JSON (Cursor): it always gets `{}`.
//! - Stay fast: parse stdin, translate, one append, exit. No network.
//! - Never trust the input: unknown events are recorded, bad input is logged.
//!
//! When a session starts, it's also linked to whatever started it, and passes
//! its own identity on to anything it starts (see `link_session`).

use std::ffi::OsString;
use std::io::{self, Read};
use std::path::Path;
use std::time::SystemTime;

use serde_json::Value;

use crate::adapter::{self, Capture, Draft};
use crate::event::{Envelope, Payload, SCHEMA_VERSION, Source, truncate_chars, truncate_strings};
use crate::{link, paths, process, store};

/// Longest line we write (but see `MAX_TASKS_LINE`): one huge label
/// shouldn't bloat the log.
pub const MAX_LINE: usize = 4096;

/// Longest line for a task list. A list is one piece of state, so it's kept
/// whole, in one event (one step on the timeline, with the right counts),
/// with more room, and past that with its items' text cut shorter. Not much
/// more, though: a todo tool sends its whole list at every change.
pub const MAX_TASKS_LINE: usize = 16 * 1024;

/// How short a task list's text is cut, in turn, until the list fits.
const TASK_TEXT_CUTS: &[usize] = &[200, 80, 40, 20, 1];

/// The emit entry point. Never returns, and always exits 0.
pub fn run_and_exit(args: &[OsString]) -> ! {
    // A panic must not print to stderr or change the exit code.
    std::panic::set_hook(Box::new(|info| log_error(&format!("panic: {info}"))));
    let replies = provider_arg(args)
        .and_then(|p| adapter::by_name(&p))
        .is_some_and(|a| a.replies_with_json());
    let _ = std::panic::catch_unwind(|| {
        if let Err(e) = run(args) {
            log_error(&e);
        }
    });
    if replies {
        use std::io::Write;
        // Nothing to ask of the provider. (Cursor's `sessionStart` can set
        // variables, but only for its later hooks, not its commands: they
        // have CURSOR_CONVERSATION_ID, which links what they start.)
        let mut out = io::stdout();
        let _ = writeln!(out, "{{}}");
        let _ = out.flush();
    }
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

    let mut translation = translation?;
    if translation.drafts.is_empty() {
        return Ok(());
    }
    let dir = paths::events_dir(&root);
    let file = dir.join(format!("{}.jsonl", translation.file_key));
    let starts = translation
        .drafts
        .iter()
        .any(|d| matches!(d.payload, Payload::SessionStarted(_)));
    if adapter.needs_start() && !starts && !file.exists() {
        match parsed.as_ref().ok().and_then(|input| adapter.start(input)) {
            Some(start) => translation.drafts.insert(0, start),
            None => return Ok(()),
        }
    }
    let env_file = adapter
        .env_file_var()
        .and_then(std::env::var_os)
        .filter(|f| !f.is_empty());
    for draft in &mut translation.drafts {
        link_session(draft, env_file.as_deref().map(Path::new));
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

    store::ensure_dir(&dir).map_err(|e| format!("emit: creating {}: {e}", dir.display()))?;
    store::append(&file, out.as_bytes())
        .map_err(|e| format!("emit: writing {}: {e}", file.display()))
}

/// For a session that's starting: links it to the session (or `agent-graph
/// run`) that started it, from `AGENT_GRAPH_PARENT`, and records its agent's
/// process and the ones above it, so the reducer can link it by process when
/// the environment didn't carry a parent. Then it passes the session's own
/// identity on through `env_file` (the provider's file of `export` lines for
/// its shell commands), so sessions it starts can link back to it.
fn link_session(draft: &mut Draft, env_file: Option<&Path>) {
    let Payload::SessionStarted(started) = &mut draft.payload else {
        return;
    };
    let var = |name: &str| std::env::var(name).ok();
    if draft.parent.is_none() {
        if let Some(parent) = link::parent_here(&draft.node) {
            started.link_method = Some(link::method_for(&parent).to_string());
            draft.parent = Some(parent);
        }
    }
    if let Some((agent, above)) = process::agent_of_this_hook() {
        started.process = Some(agent.id());
        started.ancestors = above.iter().map(process::Process::id).collect();
    }
    let traceparent = link::traceparent(&draft.node, var(link::TRACEPARENT_VAR).as_deref());
    draft.trace = Some(serde_json::json!({ "traceparent": traceparent }));
    let passed_on = [
        (link::PARENT_VAR.to_string(), draft.node.clone()),
        (
            link::PARENT_CODEX_VAR.to_string(),
            link::codex_thread_here(),
        ),
        (
            link::PARENT_CURSOR_VAR.to_string(),
            link::cursor_conversation_here(),
        ),
        (link::TRACEPARENT_VAR.to_string(), traceparent),
    ];
    if let Some(file) = env_file {
        let exports: String = passed_on
            .iter()
            .map(|(k, v)| format!("export {k}={}\n", sh_quote(v)))
            .collect();
        if let Err(e) = store::append(file, exports.as_bytes()) {
            log_error(&format!("emit: writing {}: {e}", file.display()));
        }
    }
}

/// Quotes `s` as one word for a POSIX shell.
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
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
            trace: draft.trace,
            data: draft.payload.to_data(),
        })
        .collect()
}

/// Serializes an event as one line of at most `MAX_LINE` bytes (a task
/// list, `MAX_TASKS_LINE`), shortening long strings if it has to.
pub fn to_line(event: &Envelope) -> String {
    let line = serde_json::to_string(event).unwrap_or_default();
    let tasks = event.kind == "tasks.updated";
    let max = if tasks { MAX_TASKS_LINE } else { MAX_LINE };
    if line.len() <= max {
        return line;
    }
    let mut event = event.clone();
    if tasks {
        for &cut in TASK_TEXT_CUTS {
            truncate_task_text(&mut event.data, cut);
            let line = serde_json::to_string(&event).unwrap_or_default();
            if line.len() <= max {
                return line;
            }
        }
    } else {
        truncate_strings(&mut event.data, 200);
        let line = serde_json::to_string(&event).unwrap_or_default();
        if line.len() <= max {
            return line;
        }
    }
    // Without its data the event no longer matches its type, so record it as
    // unknown rather than as a malformed event of that type.
    event.data = serde_json::json!({ "truncated": true, "type": event.kind });
    event.kind = "unknown".to_string();
    serde_json::to_string(&event).unwrap_or_default()
}

/// Cuts the text of each item in a task list's `data` to `max` characters,
/// leaving ids and statuses as they are, and the first item in progress
/// (the headline) no shorter than the first cut.
fn truncate_task_text(data: &mut Value, max: usize) {
    let Some(items) = data.get_mut("items").and_then(Value::as_array_mut) else {
        return;
    };
    let mut headline = true;
    for item in items {
        let current = item.get("status").and_then(Value::as_str) == Some("in_progress");
        let max = if current && std::mem::take(&mut headline) {
            max.max(TASK_TEXT_CUTS[0])
        } else {
            max
        };
        for key in ["text", "active_text"] {
            if let Some(Value::String(text)) = item.get_mut(key) {
                *text = truncate_chars(text, max);
            }
        }
    }
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
    use crate::event::{Payload, State, Status, TaskItem, TaskStatus, TasksUpdated};

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
                    title: None,
                    turn_end: false,
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
    fn exports_are_quoted_for_the_shell() {
        assert_eq!(sh_quote("claude-code:abc"), "'claude-code:abc'");
        assert_eq!(sh_quote("a'b; rm -rf ~"), r"'a'\''b; rm -rf ~'");
    }

    #[test]
    fn long_lines_are_shortened() {
        let summary = "x".repeat(10_000);
        let draft = Draft::new(
            "x:1",
            Payload::Status(Status {
                state: State::Idle,
                summary: Some(summary),
                title: None,
                turn_end: false,
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

    #[test]
    fn events_too_long_even_shortened_are_recorded_as_unknown() {
        let draft = Draft::new(
            "x:1",
            Payload::Status(Status {
                state: State::Idle,
                summary: None,
                title: None,
                turn_end: false,
            }),
        );
        let source = Source {
            provider: "x".into(),
            provider_version: None,
            adapter: None,
        };
        let mut event = stamp(vec![draft], &source, SystemTime::now()).remove(0);
        event.data["many"] = serde_json::json!(vec!["x"; 2000]);
        let line: Value = serde_json::from_str(&to_line(&event)).unwrap();
        assert_eq!(line["type"], "unknown");
        assert_eq!(
            line["data"],
            serde_json::json!({ "truncated": true, "type": "status" })
        );
        assert_eq!(line["id"], event.id.as_str());
    }

    /// R27: a task list too long for a line isn't dropped. It's one piece of
    /// state, so it stays one event (one step on the timeline, with the
    /// right counts) with more room, and past that its items' text is cut
    /// shorter, never their ids or statuses, nor what the headline shows.
    #[test]
    fn long_task_lists_are_kept_whole() {
        // `n` items, the middle one in progress.
        let list = |n: usize, text: usize| -> Vec<TaskItem> {
            (0..n)
                .map(|i| TaskItem {
                    id: format!("{i:04}-{}", "i".repeat(25)),
                    text: format!("{i} {}", "t".repeat(text)),
                    active_text: Some(format!("{i} {}", "a".repeat(text))),
                    status: match i {
                        _ if i == n / 2 => TaskStatus::InProgress,
                        _ if i % 2 == 0 => TaskStatus::Completed,
                        _ => TaskStatus::Pending,
                    },
                })
                .collect()
        };
        let source = Source {
            provider: "x".into(),
            provider_version: None,
            adapter: None,
        };
        let now = SystemTime::now();
        let line_of = |items: &[TaskItem]| {
            let list = Payload::TasksUpdated(TasksUpdated {
                items: items.to_vec(),
            });
            let events = stamp(vec![Draft::new("x:1", list)], &source, now);
            assert_eq!(events.len(), 1);
            to_line(&events[0])
        };
        let items_of = |line: &str| -> Vec<TaskItem> {
            let event: Envelope = serde_json::from_str(line).unwrap();
            assert_eq!(event.kind, "tasks.updated");
            serde_json::from_value::<TasksUpdated>(event.data)
                .unwrap()
                .items
        };

        // Sixty ordinary items: too long for `MAX_LINE`, and kept as they are.
        let items = list(60, 60);
        let line = line_of(&items);
        assert!(line.len() > MAX_LINE && line.len() <= MAX_TASKS_LINE);
        assert_eq!(items_of(&line), items);

        // Longer lists, or longer text: every item, with its id and status,
        // and its text cut, but only as far as it has to be, and the one in
        // progress no shorter than a label.
        for (n, text, cut) in [
            (25, 1000, 200),
            (50, 200, 80),
            (80, 200, 40),
            (110, 200, 20),
            (150, 30, 1),
        ] {
            let items = list(n, text);
            let line = line_of(&items);
            assert!(line.len() <= MAX_TASKS_LINE);
            let got = items_of(&line);
            assert_eq!(got.len(), items.len());
            for (got, want) in got.iter().zip(&items) {
                assert_eq!((&got.id, got.status), (&want.id, want.status));
                let cut = match want.status {
                    TaskStatus::InProgress => cut.max(200),
                    _ => cut,
                };
                for (got, want) in [
                    (&got.text, &want.text),
                    (
                        got.active_text.as_ref().unwrap(),
                        want.active_text.as_ref().unwrap(),
                    ),
                ] {
                    assert_eq!(got, &truncate_chars(want, cut), "{n} items of {text}");
                }
            }
        }

        // However far the rest is cut, the headline is whole.
        let items = list(150, 30);
        let event: Envelope = serde_json::from_str(&line_of(&items)).unwrap();
        let graph = crate::reducer::reduce(
            vec![event],
            &crate::reducer::Options {
                now,
                stale_after: std::time::Duration::from_secs(600),
            },
        );
        let pending = items
            .iter()
            .filter(|t| t.status == TaskStatus::Pending)
            .count();
        let current = items[75].active_text.as_deref().unwrap();
        assert_eq!(
            graph.nodes["x:1"].headline,
            Some(format!("{current} (+{pending} pending)"))
        );

        // Only the first in progress is the headline: the rest are cut too.
        let mut items = list(50, 200);
        for item in &mut items {
            item.status = TaskStatus::InProgress;
        }
        let got = items_of(&line_of(&items));
        assert_eq!(got[0].text, truncate_chars(&items[0].text, 200));
        assert!(got[1..].iter().all(|t| t.text.chars().count() < 200));

        // Beyond every cut (more than about 240 items), it's recorded as
        // unknown.
        let line: Value = serde_json::from_str(&line_of(&list(5000, 10))).unwrap();
        assert_eq!(line["type"], "unknown");
    }
}
