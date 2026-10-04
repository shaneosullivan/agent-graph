//! The graph as of any moment, and a session's timeline: the logic behind
//! both the local viewer's API and the shared site (via WebAssembly).
//! Everything here is pure: events and a clock in, JSON out.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use crate::event::{Envelope, Payload, SpawnKind, State, TaskStatus};
use crate::reducer::{self, Graph, Node, NodeKind};
use crate::resume;

/// An event with its time parsed once, kept in the order the reducer applies them.
#[derive(Debug, Clone)]
pub struct Timed {
    pub at: SystemTime,
    /// Shared, so a copy of a list of them (the local viewer's, changed
    /// while a request holds it) copies only pointers.
    pub event: Arc<Envelope>,
}

impl Timed {
    pub fn new(event: Envelope) -> Timed {
        Timed {
            at: reducer::event_time(&event),
            event: Arc::new(event),
        }
    }
}

/// Sorts events into the order the reducer applies them.
/// Sorts a log's events into the order they apply in, leaving a keyframe it
/// starts from (see `split_base`) first.
pub fn sort(events: &mut [Timed]) {
    let from = usize::from(split_base(events).0.is_some());
    events[from..].sort_by(|a, b| (a.at, &a.event.id).cmp(&(b.at, &b.event.id)));
}

/// A log's base, the keyframe it starts from, if it starts from one (as a
/// log on the site does once its start is trimmed): its first event, with
/// its parts merged. Then the rest of its events.
pub fn split_base(events: &[Timed]) -> (Option<&Envelope>, &[Timed]) {
    match events.first() {
        Some(t) if reducer::is_keyframe(&t.event) => (Some(&t.event), &events[1..]),
        _ => (None, events),
    }
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
    /// The agent it opens in: "Claude Code", or with `desktop`, the app:
    /// "the Claude app", "the ChatGPT app".
    pub app: &'static str,
    /// It's still running, so this opens a copy of its conversation.
    pub copy: bool,
    /// It opens in its desktop app, which has it, as it is.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub desktop: bool,
}

/// What the local viewer knows beyond the log, from the agents' own records.
#[derive(Default)]
pub struct Local {
    /// Session id → its name, newer than the log's.
    pub titles: BTreeMap<String, String>,
    /// Session id → the link that shows it in its desktop app, for those an
    /// app has (see `resume::Resume::desktop`).
    pub desktop: BTreeMap<String, String>,
    /// Session id → what Cursor's app has saved of the chat, newer than the
    /// log: its todo list, and whether a plan is waiting for you.
    pub cursor: BTreeMap<String, crate::adapter::cursor::AppChat>,
    /// Which of Cursor's chats are its CLI's, which it can reopen (see
    /// `resume::cursor_cli`).
    pub cursor_cli: BTreeSet<String>,
}

#[derive(Serialize)]
struct GraphResponse<'a> {
    /// The event the graph is as-of, or `None` for live.
    at: Option<&'a str>,
    /// How many events went into it.
    events: usize,
    /// Every session with no parent in the graph, most recently active first.
    roots: &'a [String],
    /// What the sessions list shows of each of `roots`.
    sessions: BTreeMap<&'a str, SessionSummary<'a>>,
    /// The node whose tree `nodes` holds: the one asked for, or if that
    /// isn't in the graph (or none was), the most recently active session.
    root: Option<&'a str>,
    /// Every node of that tree: not every node of every session, which a
    /// long history makes slow to send and to take in.
    nodes: BTreeMap<&'a str, &'a Node>,
    /// The nodes outside it that it refers to (what it waits on, say): only
    /// what names them and how they're doing.
    others: BTreeMap<&'a str, Brief<'a>>,
    /// Which of `nodes` can be reopened. Only ever filled in for
    /// `Environment::Local`.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    open: BTreeMap<&'a str, Open>,
    /// For the graph now (no `until`), that tree's timeline, as `timeline`
    /// gives it, from the same reduction: so the page refreshes with one
    /// request, and the history is reduced once.
    #[serde(skip_serializing_if = "Option::is_none")]
    stops: Option<Vec<Stop>>,
    /// For the graph now, when it will next change with nothing new
    /// happening (milliseconds since 1970), for the page to look again then.
    #[serde(skip_serializing_if = "Option::is_none")]
    recheck_ms: Option<u64>,
}

/// What names a node (as the page does), and how it's doing.
#[derive(Serialize)]
struct Brief<'a> {
    id: &'a str,
    kind: NodeKind,
    provider: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cwd: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_type: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    attention: Option<&'a str>,
    state: State,
    stale: bool,
}

impl<'a> Brief<'a> {
    fn of(n: &'a Node) -> Brief<'a> {
        Brief {
            id: &n.id,
            kind: n.kind,
            provider: &n.provider,
            parent: n.parent.as_deref(),
            title: n.title.as_deref(),
            cwd: n.cwd.as_deref(),
            agent_type: n.agent_type.as_deref(),
            attention: n.attention.as_deref(),
            state: n.state,
            stale: n.stale,
        }
    }
}

/// A session as the list shows it: itself, and what's going on in its tree.
#[derive(Serialize)]
struct SessionSummary<'a> {
    #[serde(flatten)]
    session: Brief<'a>,
    last_event_at: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    started_at: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    headline: Option<&'a str>,
    /// Its own tasks, and how many are still open.
    tasks: usize,
    open_tasks: usize,
    /// How many nodes are under it.
    agents: usize,
    /// The first node in its tree that needs you.
    needs_you: Option<Brief<'a>>,
    /// Something in its tree waits on something that waits on it.
    deadlocked: bool,
    /// The first node in its tree that looks stuck.
    stuck: Option<Brief<'a>>,
    /// Something in its tree is working.
    busy: bool,
}

impl<'a> SessionSummary<'a> {
    fn of(graph: &'a Graph, root: &'a Node) -> SessionSummary<'a> {
        let tree = tree_order(graph, &root.id);
        let first = |f: &dyn Fn(&Node) -> bool| tree.iter().copied().find(|n| f(n)).map(Brief::of);
        SessionSummary {
            session: Brief::of(root),
            last_event_at: &root.last_event_at,
            started_at: root.started_at.as_deref(),
            headline: root.headline.as_deref(),
            tasks: root.tasks.len(),
            open_tasks: root.open_tasks,
            agents: tree.len() - 1,
            needs_you: first(&|n| n.state == State::InputRequired),
            deadlocked: tree
                .iter()
                .any(|n| n.blocked.as_ref().is_some_and(|b| b.cycle)),
            stuck: first(&|n| n.stale),
            busy: tree.iter().any(|n| n.state == State::Working),
        }
    }
}

/// Every session's summary, as the list shows it (the `sessions` of a
/// `graph` reply), after all of `events`, judged at `now`: the most recently
/// active `most`, as JSON (session id → summary). `watch-remote` sends it to
/// the site, so the site can list a live share's sessions without reading
/// its log (the viewer's sidebar, at /watch).
pub fn summaries(events: &[Timed], now: SystemTime, stale_after: Duration, most: usize) -> String {
    let graph = reduce(events, now, stale_after);
    let mut roots: Vec<&Node> = graph
        .roots
        .iter()
        .filter_map(|id| graph.nodes.get(id))
        .collect();
    // (ISO 8601 times sort as text.)
    roots.sort_by(|a, b| b.last_event_at.cmp(&a.last_event_at));
    let sessions: BTreeMap<&str, SessionSummary> = roots
        .into_iter()
        .take(most)
        .map(|n| (n.id.as_str(), SessionSummary::of(&graph, n)))
        .collect();
    to_json(&sessions)
}

/// Names the sessions in `titles` (session id → name) that `graph` has.
pub fn retitle(graph: &mut Graph, titles: &BTreeMap<String, String>) {
    for (id, title) in titles {
        if let Some(node) = graph.nodes.get_mut(id) {
            node.title = Some(title.clone());
        }
    }
}

/// Puts what Cursor has saved of its chats (`Local::cursor`) on them: a todo
/// list as the chat's tasks; a chat at work that's waiting on you for an
/// approval or an answer as needing you, and, where the app's database says
/// what it's waiting for, not otherwise (not the hooks' guess); and an idle
/// chat with a plan waiting as needing you.
pub fn with_cursor(graph: &mut Graph, chats: &BTreeMap<String, crate::adapter::cursor::AppChat>) {
    for (id, chat) in chats {
        let Some(node) = graph.nodes.get_mut(id) else {
            continue;
        };
        if !chat.todos.is_empty() {
            node.tasks = chat
                .todos
                .iter()
                .map(|t| crate::reducer::Task {
                    id: t.id.clone(),
                    text: t.text.clone(),
                    active_text: t.active_text.clone(),
                    status: t.status,
                })
                .collect();
        }
        // (A turn that ended, or was stopped, while asking isn't asking.)
        let at_work = matches!(node.state, State::Working | State::InputRequired);
        match chat.waiting.filter(|_| at_work) {
            Some(waiting) => {
                node.state = State::InputRequired;
                node.attention = Some(waiting.summary().to_string());
            }
            None if chat.knows_waits
                && node.attention.as_deref() == Some(crate::reducer::MAY_ASK) =>
            {
                node.state = State::Working;
                node.attention = None;
            }
            None => {}
        }
        if chat.plan_pending && node.state == State::Idle {
            node.state = State::InputRequired;
            node.attention = Some(crate::reducer::PLAN_READY.to_string());
        }
        crate::reducer::summarize(node);
    }
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

/// `graph_at` as JSON, for a page in `env`: every session's summary, and
/// the tree under `root` (or, if it's not given or not in the graph, under
/// the most recently active session), with, for the graph now, that tree's
/// timeline.
pub fn graph(
    events: &[Timed],
    until: Option<&str>,
    root: Option<&str>,
    now: SystemTime,
    stale_after: Duration,
    env: Environment,
) -> Result<String, ApiError> {
    graph_with(
        events,
        until,
        root,
        now,
        stale_after,
        env,
        &Local::default(),
    )
}

/// `graph`, with what `local` knows: sessions' names newer than the log's,
/// and which open in a desktop app.
pub fn graph_with(
    events: &[Timed],
    until: Option<&str>,
    root: Option<&str>,
    now: SystemTime,
    stale_after: Duration,
    env: Environment,
    local: &Local,
) -> Result<String, ApiError> {
    let (mut graph, count, _) = graph_at(events, until, now, stale_after)?;
    // Names read since the log's are for now; a step back has the name the
    // session had then.
    if until.is_none() {
        retitle(&mut graph, &local.titles);
        with_cursor(&mut graph, &local.cursor);
    }
    let sessions = graph
        .roots
        .iter()
        .filter_map(|id| graph.nodes.get(id))
        .map(|n| (n.id.as_str(), SessionSummary::of(&graph, n)))
        .collect();
    let root = root
        .filter(|id| graph.nodes.contains_key(*id))
        .or(graph.roots.first().map(String::as_str));
    let nodes: BTreeMap<&str, &Node> = root
        .map(|root| {
            tree_order(&graph, root)
                .into_iter()
                .map(|n| (n.id.as_str(), n))
                .collect()
        })
        .unwrap_or_default();
    let others = neighbours(&graph, &nodes);
    let open = match env {
        Environment::Local => nodes
            .values()
            .filter_map(|n| {
                // The app shows the session itself, running or not, and
                // needs nothing else of it.
                let app = local
                    .desktop
                    .get(&n.id)
                    .and_then(|l| resume::app_of_link(l));
                let open = if let Some(app) = app {
                    Open {
                        app,
                        copy: false,
                        desktop: true,
                    }
                } else {
                    let r = resume::resume(n).or_else(|| {
                        local
                            .cursor_cli
                            .contains(&n.id)
                            .then(|| resume::cursor_cli(n))
                            .flatten()
                    })?;
                    Open {
                        app: r.app,
                        copy: r.copy,
                        desktop: false,
                    }
                };
                Some((n.id.as_str(), open))
            })
            .collect(),
        Environment::Site => BTreeMap::new(),
    };
    let stops = until
        .is_none()
        .then(|| root.map_or_else(Vec::new, |root| stops(events, &graph, root)));
    Ok(to_json(&GraphResponse {
        at: until,
        events: count,
        roots: &graph.roots,
        sessions,
        root: root
            .and_then(|id| graph.nodes.get(id))
            .map(|n| n.id.as_str()),
        nodes,
        others,
        open,
        stops,
        recheck_ms: graph
            .recheck_at
            .filter(|_| until.is_none())
            .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64),
    }))
}

/// The nodes of the tree under `root`, in the order the page walks it
/// (breadth first).
fn tree_order<'a>(graph: &'a Graph, root: &str) -> Vec<&'a Node> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    let mut queue = VecDeque::from([root]);
    while let Some(id) = queue.pop_front() {
        if let Some(node) = graph.nodes.get(id) {
            if seen.insert(node.id.as_str()) {
                out.push(node);
                queue.extend(node.children.iter().map(String::as_str));
            }
        }
    }
    out
}

/// The nodes outside `tree` that the page shows it referring to: what it's
/// waiting on (possibly in other sessions), the peers of its messages, the
/// children its requests started (normally under it, but see R54), and the
/// parent of its root.
fn neighbours<'a>(
    graph: &'a Graph,
    tree: &BTreeMap<&str, &'a Node>,
) -> BTreeMap<&'a str, Brief<'a>> {
    let mut out = BTreeMap::new();
    for node in tree.values() {
        let referred = node
            .blocked
            .iter()
            .flat_map(|b| b.on.iter().map(String::as_str))
            .chain(node.messages.iter().map(|m| m.peer.as_str()))
            .chain(node.spawns.iter().filter_map(|s| s.child.as_deref()))
            .chain(node.parent.as_deref());
        for id in referred.filter(|id| !tree.contains_key(id)) {
            if let Some(n) = graph.nodes.get(id) {
                out.insert(n.id.as_str(), Brief::of(n));
            }
        }
    }
    out
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
    let stops = stops(events, &now, root);
    Ok(to_json(&TimelineResponse { root, stops }))
}

/// Every event in the tree under `root`, as `now` (all of `events`,
/// reduced) has it, labelled. One that gives a session a new name says so.
fn stops(events: &[Timed], now: &Graph, root: &str) -> Vec<Stop> {
    let members = subtree(now, root);
    // A log that starts from a keyframe starts there, with the names it has.
    let (base, rest) = split_base(events);
    let mut titles: BTreeMap<String, String> = base
        .map(|e| {
            let opts = reducer::Options {
                now: SystemTime::UNIX_EPOCH,
                stale_after: Duration::ZERO,
            };
            reducer::reduce_from(Some(e), Vec::new(), &opts)
                .nodes
                .into_values()
                .filter_map(|n| Some((n.id, n.title?)))
                .collect()
        })
        .unwrap_or_default();
    let base = base.map(|e| Stop {
        node: root.to_string(),
        ..stop(e, now)
    });
    let rest = rest
        .iter()
        .filter(|t| members.contains(t.event.node.as_str()))
        .map(|t| {
            let e = &t.event;
            let mut stop = stop(e, now);
            if let Some(title) = new_title(e, &titles) {
                // A session's first name comes with its start; only a
                // later one is news.
                let was = titles.insert(e.node.clone(), title.clone());
                if was.is_some() || e.kind != "session.started" {
                    let verb = if was.is_some() { "renamed" } else { "named" };
                    stop.category = "lifecycle";
                    stop.label = format!("{}; {verb} “{title}”", stop.label);
                }
            }
            stop
        });
    base.into_iter().chain(rest).collect()
}

/// The name `e` gives its session, if it's not the one it had.
fn new_title(e: &Envelope, titles: &BTreeMap<String, String>) -> Option<String> {
    let title = match e.kind.as_str() {
        "status" | "session.started" => match e.payload() {
            Payload::Status(d) => d.title,
            Payload::SessionStarted(d) => d.title,
            _ => None,
        },
        _ => None,
    }?;
    (titles.get(&e.node) != Some(&title)).then_some(title)
}

fn position(events: &[Timed], id: &str) -> Result<usize, ApiError> {
    events
        .iter()
        .position(|t| t.event.id == id)
        .ok_or_else(|| ApiError::NotFound(format!("no event {id}")))
}

fn reduce(events: &[Timed], now: SystemTime, stale_after: Duration) -> Graph {
    let (base, events) = split_base(events);
    reducer::reduce_from(
        base,
        events.iter().map(|t| (*t.event).clone()).collect(),
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
        Some(node) if node.kind == NodeKind::Session => match crate::render::session_title(node) {
            Some(title) => title,
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
    match node.kind {
        NodeKind::Session => "Session".to_string(),
        NodeKind::Agent => crate::render::agent_name(node, "Agent"),
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
        Payload::Status(_) if graph.late.contains(&e.id) => (
            "status",
            format!("{who}: a late status, after the session ended"),
        ),
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
        Payload::SpawnRequested(d) if d.title.is_some() => (
            "agent",
            with_detail("Suggested a session".to_string(), d.title.as_deref()),
        ),
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
        Payload::Keyframe(_) => ("lifecycle", "Earlier history isn't included".to_string()),
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

    /// R28: a status ignored as late (just after its session ended) is
    /// labelled so, not as the session needing you.
    #[test]
    fn a_late_status_is_labelled_so() {
        use crate::adapter::Draft;
        use crate::event::{Payload, SessionEnded, SessionStarted, State, Status};
        let source = Source {
            provider: "x".into(),
            provider_version: None,
            adapter: None,
        };
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1_790_000_000);
        let started = Draft::new("x:s", Payload::SessionStarted(SessionStarted::default()));
        let ended = Draft::new("x:s", Payload::SessionEnded(SessionEnded { reason: None }));
        let asks = Draft::new(
            "x:s",
            Payload::Status(Status {
                state: State::InputRequired,
                summary: Some("Allow rm -rf?".into()),
                title: None,
                turn_end: false,
            }),
        );
        let events: Vec<Timed> = stamp(vec![started], &source, t0)
            .into_iter()
            .chain(stamp(
                vec![ended, asks],
                &source,
                t0 + Duration::from_secs(5),
            ))
            .map(Timed::new)
            .collect();
        let json: Value =
            serde_json::from_str(&timeline(&events, "x:s", SystemTime::now(), STALE).unwrap())
                .unwrap();
        let last = json["stops"].as_array().unwrap().last().unwrap();
        assert_eq!(last["category"], "status");
        assert_eq!(
            last["label"],
            "Session: a late status, after the session ended"
        );
    }

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
            &graph(
                &events,
                Some(at),
                Some(SESSION),
                SystemTime::now(),
                STALE,
                LOCAL,
            )
            .unwrap(),
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

        let live: Value = serde_json::from_str(
            &graph(
                &events,
                None,
                Some(SESSION),
                SystemTime::now(),
                STALE,
                LOCAL,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(live["at"], Value::Null);
        assert_eq!(live["nodes"][SESSION]["state"], "completed");
    }

    #[test]
    fn only_a_local_page_can_open_sessions() {
        let events = fixture_events();
        let at = |env| -> Value {
            serde_json::from_str(
                &graph(&events, None, Some(SESSION), SystemTime::now(), STALE, env).unwrap(),
            )
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
        let json: Value = serde_json::from_str(
            &graph(
                running,
                None,
                Some(SESSION),
                SystemTime::now(),
                STALE,
                LOCAL,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(json["open"][SESSION]["copy"], true);

        // The Claude desktop app has it: it opens there, as it is.
        let local = Local {
            desktop: BTreeMap::from([(
                SESSION.to_string(),
                "claude://code/continue?session=local_1".to_string(),
            )]),
            ..Local::default()
        };
        let json: Value = serde_json::from_str(
            &graph_with(
                running,
                None,
                Some(SESSION),
                SystemTime::now(),
                STALE,
                LOCAL,
                &local,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            json["open"][SESSION],
            serde_json::json!({ "app": "the Claude app", "copy": false, "desktop": true })
        );
    }

    #[test]
    fn a_session_id_that_is_not_plain_is_never_offered() {
        let line = r#"{"v":1,"id":"01K0000000000000000000000A","ts":"2026-09-25T10:00:00.000Z","type":"session.started","node":"claude-code:--dangerously-skip-permissions","source":{"provider":"claude-code"},"data":{"cwd":"/w/app"}}"#;
        let events = vec![Timed::new(serde_json::from_str(line).unwrap())];
        let root = "claude-code:--dangerously-skip-permissions";
        let json: Value = serde_json::from_str(
            &graph(&events, None, Some(root), SystemTime::now(), STALE, LOCAL).unwrap(),
        )
        .unwrap();
        assert_eq!(json["nodes"].as_object().unwrap().len(), 1);
        assert!(json.get("open").is_none());
    }

    fn events_of(lines: &[(&str, &str, &str)]) -> Vec<Timed> {
        let mut events: Vec<Timed> = lines
            .iter()
            .enumerate()
            .map(|(i, (node, kind, data))| {
                let line = format!(
                    r#"{{"v":1,"id":"01K{i:023}","ts":"2026-09-25T10:00:{i:02}.000Z","type":"{kind}","node":"{node}","data":{data}}}"#
                );
                Timed::new(serde_json::from_str(&line).unwrap())
            })
            .collect();
        sort(&mut events);
        events
    }

    /// A session's name is in its log when it changes, and a step back
    /// shows the name it had then, not one read since.
    #[test]
    fn a_rename_is_in_the_log_and_a_step_back_has_the_name_of_then() {
        let events = events_of(&[
            ("x:a", "session.started", r#"{"cwd":"/w/app"}"#),
            ("x:a", "status", r#"{"state":"working","title":"First"}"#),
            ("x:a", "status", r#"{"state":"idle","title":"First"}"#),
            ("x:a", "status", r#"{"state":"working","title":"Second"}"#),
        ]);
        let json: Value =
            serde_json::from_str(&timeline(&events, "x:a", SystemTime::now(), STALE).unwrap())
                .unwrap();
        let labels: Vec<&str> = json["stops"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["label"].as_str().unwrap())
            .collect();
        assert_eq!(
            labels,
            [
                "Session started",
                "Session is working; named “First”",
                "Session finished its turn",
                "Session is working; renamed “Second”",
            ]
        );
        assert_eq!(json["stops"][3]["category"], "lifecycle");

        let local = Local {
            titles: BTreeMap::from([("x:a".to_string(), "Third".to_string())]),
            ..Local::default()
        };
        let at = |until: Option<&str>| -> Value {
            serde_json::from_str(
                &graph_with(
                    &events,
                    until,
                    Some("x:a"),
                    SystemTime::now(),
                    STALE,
                    LOCAL,
                    &local,
                )
                .unwrap(),
            )
            .unwrap()
        };
        assert_eq!(
            at(None)["nodes"]["x:a"]["title"],
            "Third",
            "now: the latest read"
        );
        let second = events[2].event.id.clone();
        assert_eq!(at(Some(&second))["nodes"]["x:a"]["title"], "First", "then");
    }

    /// R23: a graph carries the tree it was asked for, what names the nodes
    /// it refers to outside it (not them whole), and a summary of every
    /// session for the list, rather than every node of every session.
    #[test]
    fn a_graph_carries_one_tree_and_a_summary_of_every_session() {
        let events = events_of(&[
            ("x:a", "session.started", r#"{"cwd":"/w/a"}"#),
            ("x:a/b", "agent.spawned", r#"{"agent_type":"Explore"}"#),
            (
                "x:a/b",
                "status",
                r#"{"state":"input_required","summary":"May I run it?"}"#,
            ),
            ("x:c", "session.started", r#"{"title":"The other one"}"#),
            ("x:e", "session.started", "{}"),
            ("x:c", "wait.started", r#"{"wait_id":"w","on":"x:e"}"#),
            ("x:e", "status", r#"{"state":"working"}"#),
            ("x:e", "wait.started", r#"{"wait_id":"v","on":"x:c"}"#),
            ("x:c", "wait.started", r#"{"wait_id":"u","on":"x:a/b"}"#),
            (
                "x:c",
                "message.sent",
                r#"{"message_id":"m","to":"x:a","summary":"hello"}"#,
            ),
        ]);
        let at = |root: Option<&str>| -> Value {
            serde_json::from_str(
                &graph(&events, None, root, SystemTime::now(), STALE, LOCAL).unwrap(),
            )
            .unwrap()
        };
        let keys = |v: &Value| -> Vec<String> { v.as_object().unwrap().keys().cloned().collect() };

        let c = at(Some("x:c"));
        assert_eq!(c["root"], "x:c");
        assert_eq!(keys(&c["nodes"]), ["x:c"], "x:c's tree");
        // What it waits on (an agent of x:a's, and x:e), and its message's
        // peer: named, not whole.
        assert_eq!(keys(&c["others"]), ["x:a", "x:a/b", "x:e"]);
        assert_eq!(c["others"]["x:e"]["state"], "working");
        assert!(c["others"]["x:a"].get("messages").is_none(), "not whole");
        assert_eq!(keys(&c["sessions"]), ["x:a", "x:c", "x:e"]);
        assert_eq!(c["roots"].as_array().unwrap().len(), 3);
        let a = &c["sessions"]["x:a"];
        assert_eq!(a["cwd"], "/w/a");
        assert_eq!(a["agents"], 1);
        assert_eq!(a["busy"], false);
        assert_eq!(a["needs_you"]["id"], "x:a/b");
        assert_eq!(a["needs_you"]["attention"], "May I run it?");
        assert_eq!(a["needs_you"]["agent_type"], "Explore");
        assert_eq!(c["sessions"]["x:c"]["title"], "The other one");
        assert!(c["sessions"]["x:c"]["needs_you"].is_null());
        assert_eq!(c["sessions"]["x:e"]["busy"], true);
        assert_eq!(
            c["sessions"]["x:c"]["deadlocked"], true,
            "x:c and x:e wait on each other"
        );
        assert_eq!(c["sessions"]["x:a"]["deadlocked"], false);

        // An agent's tree, with its parent named.
        let b = at(Some("x:a/b"));
        assert_eq!(keys(&b["nodes"]), ["x:a/b"]);
        assert!(
            keys(&b["others"]).contains(&"x:a".to_string()),
            "its parent"
        );

        // What's in the tree isn't named again.
        for g in [&c, &b, &at(Some("x:a"))] {
            let nodes = keys(&g["nodes"]);
            assert!(keys(&g["others"]).iter().all(|k| !nodes.contains(k)), "{g}");
        }

        // None asked for, or one not in the graph: the most recently active session's.
        let newest = c["roots"][0].as_str().unwrap().to_string();
        for asked in [None, Some("x:nope")] {
            let g = at(asked);
            assert_eq!(g["root"], newest.as_str());
            assert!(keys(&g["nodes"]).contains(&newest));
        }
    }

    /// R23: only the tree's sessions are offered to reopen.
    #[test]
    fn only_the_trees_sessions_can_be_opened() {
        let started = |id: &str| {
            let n = &id[id.len() - 1..];
            format!(
                r#"{{"v":1,"id":"01K0000000000000000000000{n}","ts":"2026-09-25T10:00:0{n}.000Z","type":"session.started","node":"claude-code:{id}","source":{{"provider":"claude-code"}},"data":{{"cwd":"/w/app"}}}}"#
            )
        };
        let one = "0d0e0f10-aaaa-4bbb-8ccc-000000000001";
        let two = "0d0e0f10-aaaa-4bbb-8ccc-000000000002";
        let mut events: Vec<Timed> = [one, two]
            .iter()
            .map(|id| Timed::new(serde_json::from_str(&started(id)).unwrap()))
            .collect();
        sort(&mut events);
        let root = format!("claude-code:{one}");
        let json: Value = serde_json::from_str(
            &graph(&events, None, Some(&root), SystemTime::now(), STALE, LOCAL).unwrap(),
        )
        .unwrap();
        assert_eq!(
            json["open"].as_object().unwrap().keys().collect::<Vec<_>>(),
            [&root]
        );
    }

    /// R23: a child a request claims is named, though it's in another
    /// session's tree (moved there by a call that session returned without
    /// having asked for it; since R54, not by one it did ask for, which
    /// takes the child from the first).
    #[test]
    fn a_child_claimed_from_outside_the_tree_is_named() {
        let events = events_of(&[
            ("x:s", "session.started", "{}"),
            // In the background, so x:s isn't waiting on it: only its
            // request refers to it.
            (
                "x:s",
                "spawn.requested",
                r#"{"call_id":"c","kind":"agent","background":true}"#,
            ),
            ("x:s/a", "agent.spawned", "{}"),
            ("x:t", "session.started", "{}"),
            (
                "x:t",
                "spawn.returned",
                r#"{"call_id":"c","child":"x:s/a"}"#,
            ),
        ]);
        let json: Value = serde_json::from_str(
            &graph(&events, None, Some("x:s"), SystemTime::now(), STALE, LOCAL).unwrap(),
        )
        .unwrap();
        let spawn = &json["nodes"]["x:s"]["spawns"][0];
        let child = spawn["child"].as_str().unwrap();
        assert!(json["nodes"].get(child).is_none(), "moved to x:t's tree");
        assert!(json["others"].get(child).is_some(), "but named");
    }

    /// R56: the graph now carries its tree's timeline, from the same
    /// reduction, so the page refreshes with one request; a step's doesn't.
    #[test]
    fn the_graph_now_carries_its_trees_timeline() {
        let events = fixture_events();
        let now = SystemTime::now();
        let at = |events: &[Timed], until: Option<&str>, root: Option<&str>| -> Value {
            serde_json::from_str(&graph(events, until, root, now, STALE, LOCAL).unwrap()).unwrap()
        };
        let agent = format!("{SESSION}/a1f00d");
        for root in [SESSION, &agent] {
            let timeline: Value =
                serde_json::from_str(&timeline(&events, root, now, STALE).unwrap()).unwrap();
            assert_eq!(at(&events, None, Some(root))["stops"], timeline["stops"]);
        }
        // None asked for: the tree the reply holds.
        let newest = at(&events, None, None);
        assert_eq!(newest["root"], SESSION);
        assert_eq!(newest["stops"].as_array().unwrap().len(), events.len());
        assert_eq!(at(&[], None, None)["stops"], serde_json::json!([]));
        assert!(
            at(&events, Some(&events[6].event.id), Some(SESSION))
                .get("stops")
                .is_none(),
            "a step's is the same timeline"
        );
    }

    #[test]
    fn unknown_ids_are_not_found() {
        let events = fixture_events();
        assert!(matches!(
            graph(&events, Some("nope"), None, SystemTime::now(), STALE, LOCAL),
            Err(ApiError::NotFound(_))
        ));
        assert!(matches!(
            timeline(&events, "x:nope", SystemTime::now(), STALE),
            Err(ApiError::NotFound(_))
        ));
    }
}
