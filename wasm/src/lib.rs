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

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::time::{Duration, SystemTime};

use agent_graph::event::Envelope;
use agent_graph::timeline::{self, ApiError, Timed};
use serde::Deserialize;

#[derive(Default)]
struct Log {
    events: Vec<Timed>,
    /// Ids already loaded; an event sent twice (e.g. a retried upload) counts once.
    seen: HashSet<String>,
    skipped: usize,
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
        until: Option<String>,
        #[serde(default)]
        now_ms: Option<f64>,
        #[serde(default = "default_stale")]
        stale_minutes: f64,
    },
    Timeline {
        root: String,
        #[serde(default)]
        now_ms: Option<f64>,
        #[serde(default = "default_stale")]
        stale_minutes: f64,
    },
    Info,
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
                until,
                now_ms,
                stale_minutes,
            } => timeline::graph(
                &log.events,
                until.as_deref(),
                now(&log, now_ms),
                minutes(stale_minutes),
            ),
            Request::Timeline {
                root,
                now_ms,
                stale_minutes,
            } => timeline::timeline(
                &log.events,
                &root,
                now(&log, now_ms),
                minutes(stale_minutes),
            ),
            Request::Info => Ok(serde_json::json!({
                "events": log.events.len(),
                "skipped": log.skipped,
                "last_event_ms": log.events.last().map(|t| ms(t.at)),
            })
            .to_string()),
        };
        match result {
            Ok(json) => json,
            Err(ApiError::NotFound(msg)) => error(404, &msg),
            Err(ApiError::Failed(msg)) => error(500, &msg),
        }
    })
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
        r#"{"v":1,"id":"01K0000000000000000000000A","ts":"2026-09-25T10:00:00.000Z","type":"session.started","node":"x:s","data":{"cwd":"/w/app"}}"#,
        "\n",
        r#"{"v":1,"id":"01K0000000000000000000000B","ts":"2026-09-25T10:00:01.000Z","type":"status","node":"x:s","data":{"state":"working"}}"#,
        "\nnot json\n",
    );

    #[test]
    fn loads_queries_and_ignores_duplicates() {
        reset();
        assert_eq!(append_text(LINES), 2);
        assert_eq!(append_text(LINES), 0, "same events again");

        let graph: serde_json::Value = serde_json::from_str(&answer(r#"{"op":"graph"}"#)).unwrap();
        assert_eq!(graph["nodes"]["x:s"]["state"], "working");
        assert_eq!(graph["events"], 2);

        let past: serde_json::Value = serde_json::from_str(&answer(
            r#"{"op":"graph","until":"01K0000000000000000000000A"}"#,
        ))
        .unwrap();
        assert_eq!(past["nodes"]["x:s"]["state"], "idle");

        let timeline: serde_json::Value =
            serde_json::from_str(&answer(r#"{"op":"timeline","root":"x:s"}"#)).unwrap();
        assert_eq!(timeline["stops"].as_array().unwrap().len(), 2);

        let info: serde_json::Value = serde_json::from_str(&answer(r#"{"op":"info"}"#)).unwrap();
        assert_eq!(info["skipped"], 2, "the bad line, once per load");

        let missing: serde_json::Value =
            serde_json::from_str(&answer(r#"{"op":"timeline","root":"x:nope"}"#)).unwrap();
        assert_eq!(missing["status"], 404);
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
