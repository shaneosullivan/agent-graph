//! The current time, as far as reading the graph is concerned.
//!
//! `AGENT_GRAPH_NOW` (RFC 3339) pins it, so saved logs such as the examples
//! look the same whenever they're opened: a session that went quiet an hour
//! before "now" still reads as hung. Recording events always uses the real
//! clock.

use std::time::SystemTime;

pub fn now() -> SystemTime {
    std::env::var("AGENT_GRAPH_NOW")
        .ok()
        .and_then(|v| humantime::parse_rfc3339_weak(v.trim()).ok())
        .unwrap_or_else(SystemTime::now)
}
