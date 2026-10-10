//! Agent Graph's reducer and timeline, compiled to WebAssembly for the shared
//! site's viewer. The site shows exactly what `agent-graph view` shows, from
//! the same code.
//!
//! There's no wasm-bindgen: strings cross the boundary as UTF-8 bytes in
//! wasm memory. The page loads events once with `append` (and again as new
//! chunks arrive), then asks `query` for graphs and timelines.
//!
//! ```js
//! const ptr = alloc(n); /* write n bytes at ptr */ append(ptr, n); dealloc(ptr, n);
//! const out = query(ptr, n); /* read result_len() bytes at out */ dealloc(out, result_len());
//! ```
//!
//! Requests are JSON:
//!
//! - `{"op": "graph", "env": "site", "root": …, "until": …, "now_ms": …, "stale_minutes": …}`:
//!   the graph now, or as of event `until`: every session's summary, and the
//!   tree under `root`, and for the graph now, that tree's timeline
//!   (`stops`), from the same reduction. `env` says where it's being shown
//!   (`"site"` or `"local"`, see `timeline::Environment`) and is required.
//! - `{"op": "info"}`
//!
//! For the site's API (site/lib/api), which needs every node, not a page's
//! view of one tree:
//!
//! - `{"op": "events"}`: the log's events, in the order they're applied
//!   (not the keyframe it may start from), each as it was sent, with its
//!   time in `ms`; and whether it starts from a keyframe (`base`).
//! - `{"op": "state", "until": …, "now_ms": …, "stale_minutes": …}`: every
//!   node, and the roots, after the events up to and including `until` (or
//!   all of them), judged at `now_ms` (or, given `until`, that event's
//!   time; otherwise the last event's).
//! - `{"op": "changes", "from": i, "to": j, "stale_minutes": …}`: the nodes
//!   as they were before event `i` (`before`), then, for each event from
//!   `i` to `j` (indexes into `events`), the nodes it changed, whole, and
//!   those it removed, each judged at its own event's time.
//!
//! Cutting a log to send to the site (its last two keyframes' worth:
//! `keyframe::Trimmer`) is done a step at a time, so a page (a worker) can
//! show how far along it is: `trim_start`, `trim_step` until it's done
//! (`trim_progress` says how far along), then `trim_finish`.
//!
//! A log that starts from a keyframe (as one on the site does once its start
//! is trimmed) starts there; a keyframe later in it stands for events
//! already loaded, and is skipped.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::time::{Duration, SystemTime};

use agent_graph::event::{Envelope, Payload};
use agent_graph::keyframe::Trimmer;
use agent_graph::reducer::{self, KEYFRAME_PARTS, Options, Replay, is_keyframe};
use agent_graph::timeline::{self, ApiError, Environment, Timed};
use serde::Deserialize;

#[derive(Default)]
struct Log {
    /// Sorted, after the keyframe the log starts from, if it does.
    events: Vec<Timed>,
    /// Ids already loaded; an event sent twice (e.g. a retried upload) counts once.
    seen: HashSet<String>,
    skipped: usize,
    /// The parts so far of the keyframe the log starts from, and whether
    /// they came to nothing (after which none is looked for).
    parts: Vec<Envelope>,
    no_base: bool,
}

thread_local! {
    static LOG: RefCell<Log> = RefCell::new(Log::default());
    static RESULT_LEN: Cell<usize> = const { Cell::new(0) };
}

/// Reserves `len` bytes for the page to write into.
#[unsafe(no_mangle)]
pub extern "C" fn alloc(len: usize) -> *mut u8 {
    Box::into_raw(vec![0u8; len].into_boxed_slice()) as *mut u8
}

/// Frees bytes from `alloc`, or a result from `query`.
///
/// # Safety
/// `ptr` and `len` must come from `alloc`, or from `query` and `result_len`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dealloc(ptr: *mut u8, len: usize) {
    if !ptr.is_null() {
        drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len)) });
    }
}

/// Adds the JSON Lines in `ptr..ptr+len` to the log. Returns how many new
/// events were added.
///
/// # Safety
/// `ptr..ptr+len` must be readable memory, e.g. from `alloc`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn append(ptr: *const u8, len: usize) -> u32 {
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    append_text(&String::from_utf8_lossy(bytes)) as u32
}

/// Forgets every event.
#[unsafe(no_mangle)]
pub extern "C" fn reset() {
    LOG.with(|log| *log.borrow_mut() = Log::default());
}

thread_local! {
    static TRIMMER: RefCell<Option<Trimmer>> = const { RefCell::new(None) };
}

/// Starts cutting the JSON Lines in `ptr..ptr+len` to send to the site, with
/// `every` events between keyframes (see `keyframe::Trimmer`). The bytes are
/// copied: free them once it returns.
///
/// # Safety
/// `ptr..ptr+len` must be readable memory, e.g. from `alloc`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn trim_start(ptr: *const u8, len: usize, every: u32) {
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    let trimmer = Trimmer::new(bytes.to_vec(), every as usize);
    TRIMMER.with(|t| *t.borrow_mut() = Some(trimmer));
}

/// Does up to `lines` lines' work. Returns 1 when it's done (or there's
/// nothing started).
#[unsafe(no_mangle)]
pub extern "C" fn trim_step(lines: u32) -> u32 {
    TRIMMER.with(|t| {
        t.borrow_mut()
            .as_mut()
            .is_none_or(|t| t.step(lines as usize)) as u32
    })
}

/// How far along it is, from 0 to 1.
#[unsafe(no_mangle)]
pub extern "C" fn trim_progress() -> f64 {
    TRIMMER.with(|t| t.borrow().as_ref().map_or(1.0, Trimmer::progress))
}

/// The log to send: a line of JSON, `{"events": <how many it sends>, "of":
/// <how many there are>}`, then its lines. Its length is `result_len()`;
/// free it with `dealloc`.
#[unsafe(no_mangle)]
pub extern "C" fn trim_finish() -> *mut u8 {
    let trimmed = TRIMMER
        .with(|t| t.borrow_mut().take())
        .map(|t| t.finish().0);
    let mut out = match &trimmed {
        Some(t) => serde_json::json!({ "events": t.events, "of": t.of }),
        None => serde_json::json!({ "events": 0, "of": 0 }),
    }
    .to_string()
    .into_bytes();
    out.push(b'\n');
    if let Some(t) = trimmed {
        out.extend(t.text);
    }
    let out = out.into_boxed_slice();
    RESULT_LEN.with(|n| n.set(out.len()));
    Box::into_raw(out) as *mut u8
}

/// Answers the JSON request in `ptr..ptr+len` and returns a pointer to the
/// JSON reply; its length is `result_len()`. Free it with `dealloc`.
///
/// # Safety
/// `ptr..ptr+len` must be readable memory, e.g. from `alloc`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn query(ptr: *const u8, len: usize) -> *mut u8 {
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    let reply = answer(&String::from_utf8_lossy(bytes))
        .into_bytes()
        .into_boxed_slice();
    RESULT_LEN.with(|n| n.set(reply.len()));
    Box::into_raw(reply) as *mut u8
}

/// The length of the last `query` reply.
#[unsafe(no_mangle)]
pub extern "C" fn result_len() -> usize {
    RESULT_LEN.with(Cell::get)
}

/// Parses and adds lines, skipping bad lines and events already seen.
pub fn append_text(text: &str) -> usize {
    LOG.with(|log| {
        let mut log = log.borrow_mut();
        let mut added = 0;
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            match serde_json::from_str::<Envelope>(line) {
                // The keyframe the log starts from, once its parts are here;
                // a later one stands for events already here.
                Ok(event) if is_keyframe(&event) => {
                    // One its sender says to start again at (an event came
                    // late, and what came before hasn't it in its place): the
                    // site now starts there, and so does this.
                    let restart =
                        matches!(event.payload(), Payload::Keyframe(k) if k.restart && k.part == 0);
                    if restart && !log.events.is_empty() {
                        let skipped = log.skipped;
                        *log = Log {
                            skipped,
                            ..Log::default()
                        };
                    }
                    if log.events.is_empty() && !log.no_base && log.seen.insert(event.id.clone()) {
                        log.parts.push(event);
                        let count = match log.parts[0].payload() {
                            Payload::Keyframe(k) => k.parts,
                            _ => 0,
                        };
                        // Merged once, when they're all here.
                        if log.parts.len() >= count.min(KEYFRAME_PARTS) {
                            match reducer::merge_keyframe(&log.parts) {
                                Some(base) => {
                                    log.events.push(Timed::new(base));
                                    added += 1;
                                }
                                None => log.no_base = true,
                            }
                            log.parts.clear();
                        }
                    }
                }
                Ok(event) if log.seen.insert(event.id.clone()) => {
                    log.events.push(Timed::new(event));
                    added += 1;
                }
                Ok(_) => {}
                Err(_) => log.skipped += 1,
            }
        }
        if added > 0 {
            timeline::sort(&mut log.events);
        }
        added
    })
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Request {
    Graph {
        /// Where the graph is being shown.
        env: Environment,
        until: Option<String>,
        /// The session whose tree to send (every session is summarised).
        #[serde(default)]
        root: Option<String>,
        #[serde(default)]
        now_ms: Option<f64>,
        #[serde(default = "default_stale")]
        stale_minutes: f64,
    },
    Info,
    Events,
    State {
        #[serde(default)]
        until: Option<String>,
        #[serde(default)]
        now_ms: Option<f64>,
        #[serde(default = "default_stale")]
        stale_minutes: f64,
    },
    Changes {
        from: usize,
        to: usize,
        #[serde(default = "default_stale")]
        stale_minutes: f64,
    },
}

fn default_stale() -> f64 {
    30.0
}

/// Handles one request. Errors come back as `{"error": …, "status": …}`.
pub fn answer(request: &str) -> String {
    let request: Request = match serde_json::from_str(request) {
        Ok(r) => r,
        Err(e) => return error(400, &format!("bad request: {e}")),
    };
    LOG.with(|log| {
        let log = log.borrow();
        let result = match request {
            Request::Graph {
                env,
                until,
                root,
                now_ms,
                stale_minutes,
            } => timeline::graph(
                &log.events,
                until.as_deref(),
                root.as_deref(),
                now(&log, now_ms),
                minutes(stale_minutes),
                env,
            ),
            Request::Info => Ok(serde_json::json!({
                "events": log.events.len(),
                "skipped": log.skipped,
                "last_event_ms": log.events.last().map(|t| ms(t.at)),
            })
            .to_string()),
            Request::Events => Ok(events_json(&log.events)),
            Request::State {
                until,
                now_ms,
                stale_minutes,
            } => state(&log, until.as_deref(), now_ms, minutes(stale_minutes)),
            Request::Changes {
                from,
                to,
                stale_minutes,
            } => changes(&log.events, from, to, minutes(stale_minutes)),
        };
        match result {
            Ok(json) => json,
            Err(ApiError::NotFound(msg)) => error(404, &msg),
            Err(ApiError::Failed(msg)) => error(500, &msg),
        }
    })
}

/// The log's events, as `{"base": …, "events": [...]}` (see `Request::Events`).
fn events_json(events: &[Timed]) -> String {
    let (base, rest) = timeline::split_base(events);
    let list: Vec<serde_json::Value> = rest
        .iter()
        .map(|t| {
            let mut e = serde_json::to_value(&*t.event).expect("serializable");
            e["ms"] = ms(t.at).into();
            e
        })
        .collect();
    serde_json::json!({ "base": base.is_some(), "events": list }).to_string()
}

/// Every node after the events up to `until` (see `Request::State`).
fn state(
    log: &Log,
    until: Option<&str>,
    now_ms: Option<f64>,
    stale_after: Duration,
) -> Result<String, ApiError> {
    let events = &log.events;
    let (slice, at) = match until {
        None => (&events[..], events.last().map(|t| t.at)),
        Some(id) => {
            let pos = events
                .iter()
                .position(|t| t.event.id == id && !is_keyframe(&t.event))
                .ok_or_else(|| ApiError::NotFound(format!("no event {id}")))?;
            (&events[..=pos], Some(events[pos].at))
        }
    };
    let now = match now_ms {
        Some(ms) if ms >= 0.0 => SystemTime::UNIX_EPOCH + Duration::from_millis(ms as u64),
        _ => at.unwrap_or(SystemTime::UNIX_EPOCH),
    };
    let (base, rest) = timeline::split_base(slice);
    let graph = reducer::reduce_from(
        base,
        rest.iter().map(|t| (*t.event).clone()).collect(),
        &Options { now, stale_after },
    );
    Ok(serde_json::json!({
        "nodes": graph.nodes,
        "roots": graph.roots,
        "events": rest.len(),
    })
    .to_string())
}

/// What each event from `from` to `to` changed (see `Request::Changes`).
fn changes(
    events: &[Timed],
    from: usize,
    to: usize,
    stale_after: Duration,
) -> Result<String, ApiError> {
    let (base, rest) = timeline::split_base(events);
    if from > to || to >= rest.len() {
        return Err(ApiError::NotFound(format!(
            "no events {from} to {to} of {}",
            rest.len()
        )));
    }
    let mut replay = Replay::new(base);
    for t in &rest[..from] {
        replay.apply(&t.event);
    }
    let then = match from {
        0 => events.first().filter(|_| base.is_some()).map(|t| t.at),
        _ => Some(rest[from - 1].at),
    }
    .unwrap_or(SystemTime::UNIX_EPOCH);
    let mut prev = replay.graph(&Options {
        now: then,
        stale_after,
    });
    let before = serde_json::to_value(&prev.nodes).expect("serializable");
    let mut out = Vec::with_capacity(to - from + 1);
    for t in &rest[from..=to] {
        replay.apply(&t.event);
        let graph = replay.graph(&Options {
            now: t.at,
            stale_after,
        });
        let changed: Vec<&reducer::Node> = graph
            .nodes
            .values()
            .filter(|n| prev.nodes.get(&n.id) != Some(*n))
            .collect();
        let removed: Vec<&str> = prev
            .nodes
            .keys()
            .filter(|id| !graph.nodes.contains_key(*id))
            .map(String::as_str)
            .collect();
        out.push(serde_json::json!({
            "id": t.event.id,
            "changed": changed,
            "removed": removed,
        }));
        prev = graph;
    }
    Ok(serde_json::json!({ "before": before, "events": out }).to_string())
}

/// The moment to judge staleness from: the page's clock for a live log, or,
/// for a finished one, its last event, so it reads as it did when shared.
/// (`SystemTime::now()` isn't available in the browser.)
fn now(log: &Log, now_ms: Option<f64>) -> SystemTime {
    match now_ms {
        Some(ms) if ms >= 0.0 => SystemTime::UNIX_EPOCH + Duration::from_millis(ms as u64),
        _ => log.events.last().map_or(SystemTime::UNIX_EPOCH, |t| t.at),
    }
}

fn ms(t: SystemTime) -> u64 {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

fn minutes(m: f64) -> Duration {
    Duration::from_secs_f64((m.max(0.0)) * 60.0)
}

fn error(status: u16, message: &str) -> String {
    serde_json::json!({ "error": message, "status": status }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINES: &str = concat!(
        r#"{"v":1,"id":"01K0000000000000000000000A","ts":"2026-09-25T10:00:00.000Z","type":"session.started","node":"claude-code:s","source":{"provider":"claude-code"},"data":{"cwd":"/w/app"}}"#,
        "\n",
        r#"{"v":1,"id":"01K0000000000000000000000B","ts":"2026-09-25T10:00:01.000Z","type":"status","node":"claude-code:s","data":{"state":"working"}}"#,
        "\nnot json\n",
    );

    #[test]
    fn loads_queries_and_ignores_duplicates() {
        reset();
        assert_eq!(append_text(LINES), 2);
        assert_eq!(append_text(LINES), 0, "same events again");

        let graph: serde_json::Value = serde_json::from_str(&answer(
            r#"{"op":"graph","env":"site","root":"claude-code:s"}"#,
        ))
        .unwrap();
        assert_eq!(graph["nodes"]["claude-code:s"]["state"], "working");
        assert_eq!(graph["sessions"]["claude-code:s"]["state"], "working");
        assert_eq!(graph["events"], 2);

        let past: serde_json::Value = serde_json::from_str(&answer(
            r#"{"op":"graph","env":"site","root":"claude-code:s","until":"01K0000000000000000000000A"}"#,
        ))
        .unwrap();
        assert_eq!(past["nodes"]["claude-code:s"]["state"], "idle");

        assert_eq!(graph["stops"].as_array().unwrap().len(), 2, "its timeline");
        assert!(past.get("stops").is_none());

        let info: serde_json::Value = serde_json::from_str(&answer(r#"{"op":"info"}"#)).unwrap();
        assert_eq!(info["skipped"], 2, "the bad line, once per load");

        let missing: serde_json::Value = serde_json::from_str(&answer(
            r#"{"op":"graph","env":"site","root":"claude-code:s","until":"nope"}"#,
        ))
        .unwrap();
        assert_eq!(missing["status"], 404);
    }

    #[test]
    fn the_page_says_where_it_is() {
        reset();
        append_text(LINES);
        let graph = |request: &str| -> serde_json::Value {
            serde_json::from_str(&answer(request)).unwrap()
        };
        assert!(
            graph(r#"{"op":"graph","env":"site"}"#)
                .get("open")
                .is_none(),
            "nothing to open on the site"
        );
        assert_eq!(
            graph(r#"{"op":"graph","env":"local","root":"claude-code:s"}"#)["open"]["claude-code:s"]
                ["app"],
            "Claude Code"
        );
        assert_eq!(graph(r#"{"op":"graph"}"#)["status"], 400, "env is required");
        assert_eq!(graph(r#"{"op":"graph","env":"moon"}"#)["status"], 400);
    }

    fn event(n: usize, node: &str, kind: &str, data: &str) -> String {
        format!(
            r#"{{"v":1,"id":"01K{n:023}","ts":"2026-09-25T10:{:02}:{:02}.000Z","type":"{kind}","node":"{node}","data":{data}}}"#,
            n / 60,
            n % 60
        ) + "\n"
    }

    /// A log with sessions coming and going, and tasks changing.
    fn busy(n: usize) -> String {
        (0..n)
            .map(|i| match i % 5 {
                0 => event(i, &format!("x:s{}", i / 10), "session.started", "{}"),
                1 => event(
                    i,
                    &format!("x:s{}", i / 10),
                    "status",
                    r#"{"state":"working"}"#,
                ),
                2 => event(
                    i,
                    &format!("x:s{}", i / 10),
                    "tasks.updated",
                    &format!(r#"{{"items":[{{"id":"1","text":"t{i}","status":"in_progress"}}]}}"#),
                ),
                3 => event(i, &format!("x:s{}/a", i / 10), "agent.spawned", "{}"),
                _ => event(
                    i,
                    &format!("x:s{}", i / 10),
                    "status",
                    r#"{"state":"idle"}"#,
                ),
            })
            .collect()
    }

    fn query(request: &str) -> serde_json::Value {
        serde_json::from_str(&answer(request)).unwrap()
    }

    /// Cuts `text` with the exports the page's worker uses, a few lines at a
    /// time: `{events, of}`, and the text to send.
    fn trim(text: &str, every: u32) -> (serde_json::Value, String) {
        unsafe { trim_start(text.as_ptr(), text.len(), every) };
        let mut last = -1.0;
        while trim_step(7) == 0 {
            let progress = trim_progress();
            assert!(
                progress >= last && progress <= 1.0,
                "{last} then {progress}"
            );
            last = progress;
        }
        let out = trim_finish();
        let bytes = unsafe {
            Box::from_raw(std::ptr::slice_from_raw_parts_mut(
                out,
                RESULT_LEN.with(Cell::get),
            ))
        };
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        let (meta, lines) = text.split_once('\n').unwrap();
        (serde_json::from_str(meta).unwrap(), lines.to_string())
    }

    /// What's cut to send, loaded again, shows the same: every session, as
    /// it stands, from the keyframe it starts at.
    #[test]
    fn a_trimmed_log_shows_the_same() {
        reset();
        append_text(&busy(47));
        let whole = query(r#"{"op":"graph","env":"site","root":"x:s4"}"#);
        let (meta, text) = trim(&busy(47), 10);
        assert_eq!(meta, serde_json::json!({"events": 17, "of": 47}));

        reset();
        append_text(&text);
        let after = query(r#"{"op":"graph","env":"site","root":"x:s4"}"#);
        assert_eq!(after["sessions"], whole["sessions"]);
        assert_eq!(after["nodes"], whole["nodes"]);
        let stops = query(r#"{"op":"graph","env":"site","root":"x:s4"}"#)["stops"].clone();
        assert_eq!(stops[0]["label"], "Earlier history isn't included");

        // Cut again: its own keyframe, and its events.
        let (meta, again) = trim(&text, 1000);
        assert_eq!(meta, serde_json::json!({"events": 17, "of": 17}));
        reset();
        append_text(&again);
        assert_eq!(
            query(r#"{"op":"graph","env":"site","root":"x:s4"}"#)["nodes"],
            whole["nodes"]
        );
    }

    /// Only a keyframe at the log's start is started from, merged once its
    /// parts are all here, whichever reads they come in; a later keyframe
    /// is skipped; and one that doesn't merge isn't tried again.
    #[test]
    fn only_a_starting_keyframe_counts() {
        let (_, text) = trim(&busy(47), 10);
        reset();
        let mut added = 0;
        for line in text.lines() {
            added += append_text(line);
        }
        assert_eq!(added, 18, "the base, and the events");
        assert_eq!(query(r#"{"op":"info"}"#)["events"], 18);
        // Events first: no keyframe is started from.
        reset();
        append_text(&busy(3));
        append_text(&text);
        let stops = query(r#"{"op":"graph","env":"site","root":"x:s0"}"#)["stops"].clone();
        assert_ne!(stops[0]["label"], "Earlier history isn't included");
        // A keyframe that doesn't merge: then no other is tried.
        let bad = text
            .lines()
            .next()
            .unwrap()
            .replace("~000000", "~999999")
            .replace(r#""text":""#, r#""text":"!"#);
        reset();
        append_text(&(bad + "\n"));
        append_text(&text);
        let stops = query(r#"{"op":"graph","env":"site","root":"x:s4"}"#)["stops"].clone();
        assert_ne!(stops[0]["label"], "Earlier history isn't included");
    }

    /// An event whose id comes right after the last one a keyframe stands
    /// for (the next from the same hook) isn't taken for the keyframe.
    #[test]
    fn the_event_after_a_keyframe_is_kept() {
        let first = event(0, "x:s", "session.started", "{}");
        let next = first
            .replace("01K00000000000000000000000", "01K00000000000000000000001")
            .replace("session.started", "status")
            .replace(r#""data":{}"#, r#""data":{"state":"input_required"}"#);
        let (_, text) = trim(&(first + &next), 1);
        reset();
        append_text(&text);
        let graph = query(r#"{"op":"graph","env":"site","root":"x:s"}"#);
        assert_eq!(graph["nodes"]["x:s"]["state"], "input_required");
    }

    /// A keyframe its sender says to start again at, later in the log, is
    /// started again from: a reader already reading has what the site now
    /// has.
    #[test]
    fn a_restart_keyframe_is_started_again_from() {
        let (_, text) = trim(&busy(47), 10);
        let restart: String = text
            .lines()
            .rfind(|l| l.contains(r#""type":"keyframe""#))
            .unwrap()
            .replace(r#""data":{"#, r#""data":{"restart":true,"#)
            + "\n";
        reset();
        append_text(&busy(3));
        append_text(&restart);
        // (Session 4 starts after it.)
        let stops = query(r#"{"op":"graph","env":"site","root":"x:s3"}"#)["stops"].clone();
        assert_eq!(stops[0]["label"], "Earlier history isn't included");
        assert_eq!(query(r#"{"op":"info"}"#)["events"], 1, "just the base");
        // Not one that isn't marked.
        reset();
        append_text(&busy(3));
        append_text(&restart.replace(r#""restart":true,"#, ""));
        assert_eq!(query(r#"{"op":"info"}"#)["events"], 3);
    }

    /// The nodes `changes` gives, applied in turn to its `before`, are the
    /// nodes `state` gives at each event: from the start of a log, partway
    /// through, and from a keyframe a trimmed log starts at.
    #[test]
    fn changes_add_up_to_each_state() {
        let (_, trimmed) = trim(&busy(47), 10);
        for text in [busy(47), trimmed] {
            reset();
            append_text(&text);
            let events = query(r#"{"op":"events"}"#)["events"].clone();
            let n = events.as_array().unwrap().len();
            for from in [0, n / 2] {
                let to = n - 1;
                let out = query(&format!(r#"{{"op":"changes","from":{from},"to":{to}}}"#));
                let mut nodes = out["before"].as_object().unwrap().clone();
                if from > 0 {
                    let id = events[from - 1]["id"].as_str().unwrap();
                    let then = query(&format!(r#"{{"op":"state","until":"{id}"}}"#));
                    assert_eq!(serde_json::Value::Object(nodes.clone()), then["nodes"]);
                }
                for (i, step) in out["events"].as_array().unwrap().iter().enumerate() {
                    let id = events[from + i]["id"].as_str().unwrap();
                    assert_eq!(step["id"], id);
                    for node in step["changed"].as_array().unwrap() {
                        nodes.insert(node["id"].as_str().unwrap().to_string(), node.clone());
                    }
                    for gone in step["removed"].as_array().unwrap() {
                        nodes.remove(gone.as_str().unwrap());
                    }
                    let want = query(&format!(r#"{{"op":"state","until":"{id}"}}"#));
                    assert_eq!(
                        serde_json::Value::Object(nodes.clone()),
                        want["nodes"],
                        "after {id}"
                    );
                }
            }
        }
    }

    /// `events` lists what's applied, in order, without the keyframe a log
    /// starts from; `state` is every node, at an event or now, judged when
    /// asked.
    #[test]
    fn events_and_state() {
        reset();
        append_text(LINES);
        let events = query(r#"{"op":"events"}"#);
        assert_eq!(events["base"], false);
        let list = events["events"].as_array().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0]["id"], "01K0000000000000000000000A");
        assert_eq!(list[0]["type"], "session.started");
        assert_eq!(list[0]["ms"], 1_790_330_400_000u64);
        assert_eq!(list[1]["data"]["state"], "working");

        let now = query(r#"{"op":"state"}"#);
        assert_eq!(now["nodes"]["claude-code:s"]["state"], "working");
        assert_eq!(now["roots"], serde_json::json!(["claude-code:s"]));
        assert_eq!(now["events"], 2);
        let first = query(r#"{"op":"state","until":"01K0000000000000000000000A"}"#);
        assert_eq!(first["nodes"]["claude-code:s"]["state"], "idle");
        assert_eq!(first["events"], 1);
        // Judged an hour later, working and silent is stale.
        let later = query(r#"{"op":"state","now_ms":1790334001000}"#);
        assert_eq!(later["nodes"]["claude-code:s"]["stale"], true);
        assert_eq!(now["nodes"]["claude-code:s"]["stale"], false);
        assert_eq!(query(r#"{"op":"state","until":"nope"}"#)["status"], 404);

        let (_, trimmed) = trim(&busy(47), 10);
        reset();
        append_text(&trimmed);
        let events = query(r#"{"op":"events"}"#);
        assert_eq!(events["base"], true);
        assert_eq!(events["events"].as_array().unwrap().len(), 17);
        assert!(
            events["events"]
                .as_array()
                .unwrap()
                .iter()
                .all(|e| e["type"] != "keyframe")
        );
        assert_eq!(query(r#"{"op":"changes","from":3,"to":2}"#)["status"], 404);
        assert_eq!(query(r#"{"op":"changes","from":0,"to":17}"#)["status"], 404);
    }

    /// An event that changes nothing (the same status again) changes no node.
    #[test]
    fn a_repeat_changes_nothing() {
        reset();
        append_text(LINES);
        append_text(
            &LINES
                .replace("0000000B", "0000000C")
                .replace("10:00:01", "10:00:02"),
        );
        let out = query(r#"{"op":"changes","from":1,"to":2}"#);
        let steps = out["events"].as_array().unwrap();
        assert_eq!(steps[0]["changed"].as_array().unwrap().len(), 1);
        // (Its last event moved on, so the node changed: but only that.)
        assert_eq!(
            steps[1]["changed"][0]["last_event_at"],
            "2026-09-25T10:00:02.000Z"
        );
        assert_eq!(steps[1]["removed"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn memory_round_trip() {
        let ptr = alloc(5);
        unsafe {
            std::ptr::copy_nonoverlapping(b"hello".as_ptr(), ptr, 5);
            assert_eq!(std::slice::from_raw_parts(ptr, 5), b"hello");
            dealloc(ptr, 5);
        }
    }
}
