//! Linking a session to the session (or `agent-graph run`) that started it,
//! when they're separate processes. See docs/design.md §6.
//!
//! A session passes its identity down through the environment: its agent's
//! shell gets `AGENT_GRAPH_PARENT=<node id>` and a W3C `TRACEPARENT`, and a
//! child session's own hooks read them back. Everything here is pure.

/// The node id of the session that started this process.
pub const PARENT_VAR: &str = "AGENT_GRAPH_PARENT";
/// W3C Trace Context, as OpenTelemetry passes it to child processes.
pub const TRACEPARENT_VAR: &str = "TRACEPARENT";

/// The parent named by `value` (from `AGENT_GRAPH_PARENT`), if it's a
/// well-formed node id other than `own`. A session's own later hooks can see
/// its own id there, which isn't a parent.
pub fn parent_from(value: Option<&str>, own: &str) -> Option<String> {
    let value = value?.trim();
    let (provider, rest) = value.split_once(':')?;
    let well_formed = value.len() <= 300
        && provider.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && provider
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !rest.is_empty()
        && rest.split('/').all(|part| !part.is_empty())
        && !value.chars().any(char::is_control);
    (well_formed && value != own).then(|| value.to_string())
}

/// How a session found its parent, recorded as `link_method`.
pub fn method_for(parent: &str) -> &'static str {
    if parent.starts_with("run:") {
        "run"
    } else {
        "env"
    }
}

/// This node's `traceparent`. It continues the trace in `inherited` (the
/// `TRACEPARENT` it was started with) when there is one, else starts one.
/// The ids come from the node id, so the same node always gets the same one.
pub fn traceparent(node: &str, inherited: Option<&str>) -> String {
    let trace_id = inherited
        .and_then(trace_id_of)
        .unwrap_or_else(|| format!("{:016x}{:016x}", fnv(node, 1), fnv(node, 2)));
    format!("00-{trace_id}-{:016x}-01", fnv(node, 3))
}

/// The trace id in a valid `traceparent`.
fn trace_id_of(traceparent: &str) -> Option<String> {
    let parts: Vec<&str> = traceparent.trim().split('-').collect();
    let hex = |s: &str, len: usize| {
        s.len() == len
            && s.chars()
                .all(|c| c.is_ascii_digit() || matches!(c, 'a'..='f'))
    };
    let [version, trace, span, flags] = parts[..] else {
        return None;
    };
    let valid = hex(version, 2)
        && version != "ff"
        && hex(trace, 32)
        && trace.chars().any(|c| c != '0')
        && hex(span, 16)
        && hex(flags, 2);
    valid.then(|| trace.to_string())
}

/// FNV-1a, seeded so one input gives several independent ids. Never zero,
/// which W3C reserves as invalid.
fn fnv(s: &str, seed: u64) -> u64 {
    let hash = s
        .bytes()
        .chain(seed.to_le_bytes())
        .fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
        });
    hash.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_parent_must_be_a_node_id_and_not_yourself() {
        let own = "claude-code:child";
        assert_eq!(
            parent_from(Some("claude-code:abc"), own).as_deref(),
            Some("claude-code:abc")
        );
        assert_eq!(
            parent_from(Some(" run:01K0 \n"), own).as_deref(),
            Some("run:01K0")
        );
        assert_eq!(
            parent_from(Some("claude-code:abc/agent1"), own).as_deref(),
            Some("claude-code:abc/agent1")
        );
        for bad in [
            "",
            "claude-code:child",
            "no-colon",
            ":abc",
            "Claude:abc",
            "claude-code:",
            "claude-code:a//b",
            "x:a\u{1b}[2J",
        ] {
            assert_eq!(parent_from(Some(bad), own), None, "{bad:?}");
        }
        assert_eq!(parent_from(None, own), None);
    }

    #[test]
    fn the_link_method_follows_the_parent() {
        assert_eq!(method_for("run:01K0"), "run");
        assert_eq!(method_for("claude-code:abc"), "env");
    }

    #[test]
    fn traceparents_are_valid_and_continue_the_inherited_trace() {
        let own = traceparent("claude-code:a", None);
        let trace = trace_id_of(&own).expect("valid");
        assert_eq!(own, traceparent("claude-code:a", None), "stable");

        let child = traceparent("claude-code:b", Some(&own));
        assert_eq!(trace_id_of(&child).as_deref(), Some(trace.as_str()));
        assert_ne!(child, own, "its own span");

        let fresh = traceparent(
            "claude-code:b",
            Some("00-00000000000000000000000000000000-0000000000000001-01"),
        );
        assert_ne!(
            trace_id_of(&fresh).as_deref(),
            Some("00000000000000000000000000000000")
        );
        assert_eq!(trace_id_of("garbage"), None);
        assert_eq!(
            trace_id_of("00-ABCDEF00000000000000000000000000-0000000000000001-01"),
            None
        );
    }
}
