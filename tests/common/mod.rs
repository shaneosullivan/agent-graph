#![allow(dead_code)]

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use agent_graph::adapter::{self, Capture};
use agent_graph::emit::stamp;
use agent_graph::event::{Envelope, Source};
use agent_graph::reducer::{self, Graph};
use serde_json::Value;

pub const SESSION: &str = "claude-code:5f2c1e8a-3b4d-4e5f-8a9b-0c1d2e3f4a5b";

pub fn agent(id: &str) -> String {
    format!("{SESSION}/{id}")
}

/// A fixed starting time, so tests are repeatable.
pub fn t0() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_790_000_000)
}

pub fn fixture(path: &str) -> Vec<Value> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(path);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("fixture line is JSON"))
        .collect()
}

/// Runs each payload through the Claude Code adapter, one second apart.
pub fn translate(payloads: &[Value], capture: Capture) -> Vec<Envelope> {
    let adapter = adapter::by_name("claude-code").unwrap();
    let source = Source {
        provider: adapter.provider().into(),
        provider_version: None,
        adapter: Some(adapter.adapter_id().into()),
    };
    payloads
        .iter()
        .enumerate()
        .flat_map(|(i, p)| {
            let t = adapter.translate(p, capture).expect("translates");
            stamp(t.drafts, &source, t0() + Duration::from_secs(i as u64))
        })
        .collect()
}

pub fn reduce(events: Vec<Envelope>) -> Graph {
    reducer::reduce(
        events,
        &reducer::Options {
            now: t0() + Duration::from_secs(60),
            stale_after: Duration::from_secs(30 * 60),
        },
    )
}

/// The graph after the first `n` payloads of the session fixture.
pub fn session_after(n: usize) -> Graph {
    let payloads = fixture("claude-code/session.jsonl");
    reduce(translate(&payloads[..n], Capture::default()))
}
