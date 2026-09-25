//! The graph as of any moment, and a session's timeline: the logic behind
//! both the local viewer's API and the shared site (via WebAssembly).
//! Everything here is pure: events and a clock in, JSON out.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use crate::event::{Envelope, Payload, SpawnKind, State, TaskStatus};
use crate::reducer::{self, Graph, Node, NodeKind};
use crate::resume;

/// An event with its time parsed once, kept in the order the reducer applies them.
#[derive(Debug, Clone)]
pub struct Timed {
    pub at: SystemTime,
    pub event: Envelope,
}

impl Timed {
    pub fn new(event: Envelope) -> Timed {
        Timed {
            at: reducer::event_time(&event),
            event,
        }
    }
}

/// Sorts events into the order the reducer applies them.
pub fn sort(events: &mut [Timed]) {
    events.sort_by(|a, b| (a.at, &a.event.id).cmp(&(b.at, &b.event.id)));
}

#[derive(Debug, PartialEq)]
pub enum ApiError {
    NotFound(String),
    Failed(String),
}

/// Where a graph is being shown, which decides what the page can offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Environment {
    /// `agent-graph view`, on the computer the sessions ran on: sessions can
    /// be reopened in their agent.
    Local,
    /// The shared site, away from the sessions: nothing can be opened.
    Site,
}

/// A session the page can reopen (see `resume`).
#[derive(Serialize, Debug, PartialEq)]
pub struct Open {
    /// The agent it opens in: "Claude Code".
    pub app: &'static str,
    /// It's still running, so this opens a copy of its conversation.
    pub copy: bool,
}

#[derive(Serialize)]
struct GraphResponse<'a> {
    /// The event the graph is as-of, or `None` for live.
    at: Option<&'a str>,
    /// How many events went into it.
    events: usize,
    #[serde(flatten)]
    graph: Graph,
    /// Sessions that can be reopened, by node id. Only ever filled in for
    /// `Environment::Local`.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    open: BTreeMap<String, Open>,
}

/// The graph after every event up to and including `until` (or all of them,
/// judged at `now`). When looking back, staleness is judged from that moment.
pub fn graph_at(
    events: &[Timed],
    until: Option<&str>,
    now: SystemTime,
    stale_after: Duration,
) -> Result<(Graph, usize, SystemTime), ApiError> {
    let (slice, now) = match until {
        None => (events, now),
        Some(id) => {
            let pos = position(events, id)?;
            (&events[..=pos], events[pos].at)
        }
    };
    Ok((reduce(slice, now, stale_after), slice.len(), now))
}

/// `graph_at` as JSON, for a page in `env`.
pub fn graph(
    events: &[Timed],
    until: Option<&str>,
    now: SystemTime,
    stale_after: Duration,
    env: Environment,
) -> Result<String, ApiError> {
    let (graph, count, _) = graph_at(events, until, now, stale_after)?;
    let open = match env {
        Environment::Local => graph
            .nodes
            .values()
            .filter_map(|n| {
                let r = resume::resume(n)?;
                Some((
                    n.id.clone(),
                    Open {
                        app: r.app,
                        copy: r.copy,
                    },
                ))
            })
            .collect(),
        Environment::Site => BTreeMap::new(),
    };
    Ok(to_json(&GraphResponse {
        at: until,
        events: count,
        graph,
        open,
    }))
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Stop {
    /// The event id; pass it back as `until` to see the graph at this stop.
    pub id: String,
    pub ts: String,
    pub node: String,
    #[serde(rename = "type")]
    pub kind: String,
    /// Groups stops for colouring: lifecycle, agent, task, attention, status, wait, message, other.
    pub category: &'static str,
    pub label: String,
}

#[derive(Serialize)]
struct TimelineResponse<'a> {
    root: &'a str,
    stops: Vec<Stop>,
}

/// Every event in the tree under `root` (as the tree stands now), in order.
pub fn timeline(
    events: &[Timed],
    root: &str,
    now: SystemTime,
    stale_after: Duration,
) -> Result<String, ApiError> {
    let now = reduce(events, now, stale_after);
    if !now.nodes.contains_key(root) {
        return Err(ApiError::NotFound(format!("no node {root}")));
    }
    let members = subtree(&now, root);
    let stops = events
        .iter()
        .filter(|t| members.contains(t.event.node.as_str()))
        .map(|t| stop(&t.event, &now))
        .collect();
    Ok(to_json(&TimelineResponse { root, stops }))
}

fn position(events: &[Timed], id: &str) -> Result<usize, ApiError> {
    events
        .iter()
        .position(|t| t.event.id == id)
        .ok_or_else(|| ApiError::NotFound(format!("no event {id}")))
}

fn reduce(events: &[Timed], now: SystemTime, stale_after: Duration) -> Graph {
    reducer::reduce(
        events.iter().map(|t| t.event.clone()).collect(),
        &reducer::Options { now, stale_after },
    )
}

fn to_json(value: &impl Serialize) -> String {
    serde_json::to_string(value).expect("serializable")
}

fn subtree<'a>(graph: &'a Graph, root: &'a str) -> BTreeSet<&'a str> {
    let mut seen = BTreeSet::new();
    let mut queue = VecDeque::from([root]);
    while let Some(id) = queue.pop_front() {
        if let Some(node) = graph.nodes.get(id) {
            if seen.insert(node.id.as_str()) {
                queue.extend(node.children.iter().map(String::as_str));
            }
        }
    }
    seen
}

fn stop(e: &Envelope, graph: &Graph) -> Stop {
    let (category, label) = describe(e, graph);
    Stop {
        id: e.id.clone(),
        ts: e.ts.clone(),
        node: e.node.clone(),
        kind: e.kind.clone(),
        category,
        label,
    }
}

/// A name for a node referred to from somewhere else: sessions by folder.
fn other_name(graph: &Graph, id: &str) -> String {
    match graph.nodes.get(id) {
        Some(node) if node.kind == NodeKind::Session => match &node.title {
            Some(title) => title.clone(),
            None => node
                .cwd
                .as_deref()
                .and_then(|c| c.rsplit(['/', '\\']).find(|s| !s.is_empty()))
                .map_or_else(|| name(graph, id), |dir| format!("session {dir}")),
        },
        _ => name(graph, id),
    }
}

/// A short name for a node: the agent type and id, or "Session".
pub fn name(graph: &Graph, id: &str) -> String {
    match graph.nodes.get(id) {
        Some(node) => node_name(node),
        None => short(id.rsplit(['/', ':']).next().unwrap_or(id)).to_string(),
    }
}

fn node_name(node: &Node) -> String {
    let local = node.id.rsplit(['/', ':']).next().unwrap_or(&node.id);
    match node.kind {
        NodeKind::Session => "Session".to_string(),
        NodeKind::Agent => format!(
            "{} {}",
            node.agent_type.as_deref().unwrap_or("Agent"),
            short(local)
        ),
    }
}

fn short(id: &str) -> &str {
    id.char_indices().nth(8).map_or(id, |(i, _)| &id[..i])
}

/// A category and a one-line, human description of what an event did.
pub fn describe(e: &Envelope, graph: &Graph) -> (&'static str, String) {
    let who = name(graph, &e.node);
    let node = graph.nodes.get(&e.node);
    let task_text = |id: &str, text: Option<String>| {
        text.or_else(|| {
            node?
                .tasks
                .iter()
                .find(|t| t.id == id)
                .map(|t| t.text.clone())
        })
        .unwrap_or_else(|| format!("task {id}"))
    };

    match e.payload() {
        Payload::SessionStarted(d) => (
            "lifecycle",
            match (d.source.as_deref(), &e.parent) {
                (None | Some("startup" | "run"), Some(parent)) => {
                    format!("Session started by {}", other_name(graph, parent))
                }
                (None | Some("startup" | "run"), None) => "Session started".to_string(),
                (Some("resume"), _) => "Session resumed".to_string(),
                (Some("clear"), _) => "Conversation cleared".to_string(),
                (Some("compact"), _) => "Context compacted".to_string(),
                (Some(other), _) => format!("Session started ({other})"),
            },
        ),
        Payload::SessionEnded(_) => ("lifecycle", "Session ended".to_string()),
        Payload::AgentSpawned(_) => {
            let purpose = node.and_then(|n| n.purpose.as_deref());
            ("agent", with_detail(format!("{who} started"), purpose))
        }
        Payload::AgentFinished(d) => {
            let verb = match d.status {
                crate::event::FinishStatus::Completed => "finished",
                crate::event::FinishStatus::Failed => "failed",
                crate::event::FinishStatus::Canceled => "was canceled",
            };
            ("agent", format!("{who} {verb}"))
        }
        Payload::Status(d) => match d.state {
            State::InputRequired => (
                "attention",
                with_detail(format!("{who} needs you"), d.summary.as_deref()),
            ),
            State::Working => ("status", format!("{who} is working")),
            State::Idle => ("status", format!("{who} finished its turn")),
            State::Completed => ("status", format!("{who} completed")),
            State::Failed => ("status", format!("{who} failed")),
            State::Canceled => ("status", format!("{who} was canceled")),
        },
        Payload::TasksUpdated(d) => {
            let done = d
                .items
                .iter()
                .filter(|t| t.status == TaskStatus::Completed)
                .count();
            (
                "task",
                format!("Task list updated: {done} of {} done", d.items.len()),
            )
        }
        Payload::TaskUpserted(d) => {
            let created = d.status == Some(TaskStatus::Pending) && d.text.is_some();
            let text = task_text(&d.id, d.text);
            let label = match (created, d.status) {
                (true, _) => format!("Task added: {text}"),
                (_, Some(TaskStatus::InProgress)) => format!("Started: {text}"),
                (_, Some(TaskStatus::Completed)) => format!("Done: {text}"),
                _ => format!("Task updated: {text}"),
            };
            ("task", label)
        }
        Payload::TaskDeleted(d) => ("task", format!("Task removed: {}", task_text(&d.id, None))),
        Payload::SpawnRequested(d) => {
            let what = match d.kind {
                SpawnKind::Agent => d
                    .agent_type
                    .map_or("an agent".to_string(), |t| format!("{t} agent")),
                SpawnKind::Session => d
                    .agent_type
                    .map_or("a session".to_string(), |t| format!("a {t} session")),
            };
            let mut label = with_detail(format!("Asked for {what}"), d.purpose.as_deref());
            if d.background {
                label.push_str(" (background)");
            }
            ("agent", label)
        }
        Payload::SpawnReturned(d) => {
            let spawn = node.and_then(|n| n.spawns.iter().find(|s| s.call_id == d.call_id));
            let session = spawn.is_some_and(|s| s.kind == SpawnKind::Session);
            // A session started from the shell is paired by the reducer.
            let child = d
                .child
                .as_deref()
                .or_else(|| spawn.and_then(|s| s.child.as_deref()))
                .map(|c| {
                    if session {
                        other_name(graph, c)
                    } else {
                        name(graph, c)
                    }
                });
            let label = match (child, d.outcome.as_deref(), session) {
                (Some(c), Some("async_launched"), _) => format!("{c} launched in the background"),
                (Some(c), _, true) if spawn.is_some_and(|s| s.background) => {
                    format!("{c} launched in the background")
                }
                (Some(c), _, true) => format!("{c} finished, and the command returned"),
                (Some(c), _, false) => format!("{c} returned its result"),
                (None, _, true) => "The command returned".to_string(),
                (None, _, false) => "Agent call returned".to_string(),
            };
            ("agent", label)
        }
        Payload::WaitStarted(d) => ("wait", format!("Waiting on {}", other_name(graph, &d.on))),
        Payload::WaitEnded(_) => ("wait", "Stopped waiting".to_string()),
        Payload::MessageSent(d) => {
            let to = if graph.nodes.contains_key(&d.to) {
                other_name(graph, &d.to)
            } else {
                // Recipients are often the provider's own id for an agent.
                let session = e.node.split('/').next().unwrap_or(&e.node);
                let candidate = format!("{session}/{}", d.to);
                if graph.nodes.contains_key(&candidate) {
                    name(graph, &candidate)
                } else {
                    d.to.clone()
                }
            };
            (
                "message",
                with_detail(format!("Message to {to}"), d.summary.as_deref()),
            )
        }
        Payload::Activity(d) => ("other", format!("Used {}", d.tool)),
        Payload::Unknown(_) => ("other", "Unrecognised event".to_string()),
    }
}

fn with_detail(head: String, detail: Option<&str>) -> String {
    match detail {
        Some(d) if !d.is_empty() => format!("{head}: {d}"),
        _ => head,
    }
}

#[cfg(all(test, feature = "cli"))]
mod tests {
    use super::*;
    use crate::emit::stamp;
    use crate::event::Source;
    use serde_json::Value;

    /// The session fixture, as the viewer would hold it.
    fn fixture_events() -> Vec<Timed> {
        let adapter = crate::adapter::by_name("claude-code").unwrap();
        let source = Source {
            provider: "claude-code".into(),
            provider_version: None,
            adapter: None,
        };
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1_790_000_000);
        let text = include_str!("../tests/fixtures/claude-code/session.jsonl");
        text.lines()
            .enumerate()
            .flat_map(|(i, line)| {
                let payload: Value = serde_json::from_str(line).unwrap();
                let t = adapter.translate(&payload, Default::default()).unwrap();
                stamp(t.drafts, &source, t0 + Duration::from_secs(i as u64))
            })
            .map(Timed::new)
            .collect()
    }

    const SESSION: &str = "claude-code:5f2c1e8a-3b4d-4e5f-8a9b-0c1d2e3f4a5b";
    const STALE: Duration = Duration::from_secs(1800);
    const LOCAL: Environment = Environment::Local;

    #[test]
    fn timeline_labels_every_event_in_the_tree() {
        let events = fixture_events();
        let json: Value =
            serde_json::from_str(&timeline(&events, SESSION, SystemTime::now(), STALE).unwrap())
                .unwrap();
        let labels: Vec<&str> = json["stops"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["label"].as_str().unwrap())
            .collect();
        assert_eq!(
            labels.len(),
            events.len(),
            "every fixture event belongs to the session"
        );
        assert_eq!(
            labels,
            [
                "Session started",
                "Session is working",
                "Task added: Find the auth code",
                "Task added: Fix the token refresh bug",
                "Started: Find the auth code",
                "Asked for Explore agent: Find the auth middleware",
                "Explore a1f00d started: Find the auth middleware",
                "Explore a1f00d finished",
                "Explore a1f00d returned its result",
                "Asked for general-purpose agent: Run the test suite (background)",
                "general-purpose b2c0de launched in the background",
                "general-purpose b2c0de started: Run the test suite",
                "Message to general-purpose b2c0de: Also run lint",
                "Session needs you: Claude needs your permission to use Bash",
                "Done: Find the auth code",
                "Session finished its turn",
                "general-purpose b2c0de finished",
                "Session ended",
            ]
        );
        let categories: BTreeSet<&str> = json["stops"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["category"].as_str().unwrap())
            .collect();
        assert!(categories.contains("attention"));
    }

    #[test]
    fn timeline_of_an_agent_covers_just_its_subtree() {
        let events = fixture_events();
        let agent = format!("{SESSION}/a1f00d");
        let json: Value =
            serde_json::from_str(&timeline(&events, &agent, SystemTime::now(), STALE).unwrap())
                .unwrap();
        assert_eq!(json["stops"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn graph_at_a_stop_matches_the_events_so_far() {
        let events = fixture_events();
        // Stop 6 is the Explore agent starting: the session is blocked on it.
        let at = &events[6].event.id;
        let json: Value = serde_json::from_str(
            &graph(&events, Some(at), SystemTime::now(), STALE, LOCAL).unwrap(),
        )
        .unwrap();
        assert_eq!(json["at"], at.as_str());
        assert_eq!(json["events"], 7);
        let session = &json["nodes"][SESSION];
        assert_eq!(session["blocked"]["on"][0], format!("{SESSION}/a1f00d"));
        assert!(
            json["nodes"].get(format!("{SESSION}/b2c0de")).is_none(),
            "not started yet"
        );

        let live: Value =
            serde_json::from_str(&graph(&events, None, SystemTime::now(), STALE, LOCAL).unwrap())
                .unwrap();
        assert_eq!(live["at"], Value::Null);
        assert_eq!(live["nodes"][SESSION]["state"], "completed");
    }

    #[test]
    fn only_a_local_page_can_open_sessions() {
        let events = fixture_events();
        let at = |env| -> Value {
            serde_json::from_str(&graph(&events, None, SystemTime::now(), STALE, env).unwrap())
                .unwrap()
        };
        let local = at(Environment::Local);
        let open = local["open"].as_object().unwrap();
        assert_eq!(
            open.keys().collect::<Vec<_>>(),
            [SESSION],
            "the session, not its agents"
        );
        assert_eq!(
            open[SESSION],
            serde_json::json!({ "app": "Claude Code", "copy": false }),
            "it ended, so it's resumed rather than copied"
        );
        assert!(at(Environment::Site).get("open").is_none());

        // Still running: open a copy, not a second process on it.
        let running = &events[..events.len() - 1];
        let json: Value =
            serde_json::from_str(&graph(running, None, SystemTime::now(), STALE, LOCAL).unwrap())
                .unwrap();
        assert_eq!(json["open"][SESSION]["copy"], true);
    }

    #[test]
    fn a_session_id_that_is_not_plain_is_never_offered() {
        let line = r#"{"v":1,"id":"01K0000000000000000000000A","ts":"2026-09-25T10:00:00.000Z","type":"session.started","node":"claude-code:--dangerously-skip-permissions","source":{"provider":"claude-code"},"data":{"cwd":"/w/app"}}"#;
        let events = vec![Timed::new(serde_json::from_str(line).unwrap())];
        let json: Value =
            serde_json::from_str(&graph(&events, None, SystemTime::now(), STALE, LOCAL).unwrap())
                .unwrap();
        assert_eq!(json["nodes"].as_object().unwrap().len(), 1);
        assert!(json.get("open").is_none());
    }

    #[test]
    fn unknown_ids_are_not_found() {
        let events = fixture_events();
        assert!(matches!(
            graph(&events, Some("nope"), SystemTime::now(), STALE, LOCAL),
            Err(ApiError::NotFound(_))
        ));
        assert!(matches!(
            timeline(&events, "x:nope", SystemTime::now(), STALE),
            Err(ApiError::NotFound(_))
        ));
    }
}
