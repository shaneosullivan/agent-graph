//! Replays events into the current graph.
//!
//! The reducer is the only place that keeps history, so it resolves anything
//! that depends on more than one event: binding a spawn request to the child
//! it started, merging task updates, and working out who is blocked on whom.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::{Duration, SystemTime};

use serde::Serialize;

use crate::event::{Envelope, FinishStatus, Payload, SpawnKind, State, TaskStatus};

#[derive(Debug, Clone)]
pub struct Options {
    /// The time to measure staleness against.
    pub now: SystemTime,
    /// A working node with no events for this long, and nothing it's waiting
    /// on, is flagged `stale`: it may have hung or crashed.
    pub stale_after: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            now: crate::clock::now(),
            stale_after: Duration::from_secs(30 * 60),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Graph {
    pub nodes: BTreeMap<String, Node>,
    /// Nodes with no known parent, most recently active first.
    pub roots: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Session,
    Agent,
}

#[derive(Debug, Clone, Serialize)]
pub struct Node {
    pub id: String,
    pub kind: NodeKind,
    pub provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    pub children: Vec<String>,
    pub state: State,
    /// The node's own status line, when it reports one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// What the human is being asked, while `state` is `input_required`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attention: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    /// Why the node was started, from the spawn request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<bool>,
    /// The spawn request (`call_id`) this node was bound to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spawned_by: Option<String>,
    /// For a session started by another: how the link was found. `env` (it
    /// inherited its parent's identity), `run` (`agent-graph run` started
    /// it) or `process` (its parent's agent is among its processes).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    /// The session's agent process, `<pid>@<start>`, when recorded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process: Option<String>,
    pub tasks: Vec<Task>,
    pub spawns: Vec<Spawn>,
    pub waits: Vec<Wait>,
    pub messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
    pub last_event_at: String,

    // Derived in `finish`.
    /// One line on what's happening now: the status line, else the task in progress.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headline: Option<String>,
    pub open_tasks: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked: Option<Blocked>,
    pub stale: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Task {
    pub id: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_text: Option<String>,
    pub status: TaskStatus,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Spawn {
    pub call_id: String,
    pub kind: SpawnKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    pub background: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub child: Option<String>,
    pub returned: bool,
    pub requested_at: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Wait {
    pub wait_id: String,
    /// The node waited on. `None` while a requested child hasn't started yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub open: bool,
    /// For a child this node asked for. These end when the spawning call
    /// returns, which the provider reports exactly.
    pub spawn: bool,
    pub started_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Sent,
    Received,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Message {
    pub message_id: String,
    pub direction: Direction,
    /// The other side: a node id when we could resolve it, else the provider's name for it.
    pub peer: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    pub ts: String,
}

/// What stands between a node and continuing.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Blocked {
    /// Nodes this node is directly waiting on.
    pub on: Vec<String>,
    /// Requested children that haven't started yet.
    pub starting: usize,
    /// Every unfinished node it's transitively waiting on, including their children.
    pub nodes: usize,
    /// Open tasks across those nodes.
    pub open_tasks: usize,
    /// The waits lead back to this node: nothing in the loop can finish.
    pub cycle: bool,
}

/// When an event happened, for ordering. Unparseable times sort first.
pub fn event_time(e: &Envelope) -> SystemTime {
    humantime::parse_rfc3339_weak(&e.ts).unwrap_or(SystemTime::UNIX_EPOCH)
}

/// Events apply in time order, then id order. Ids from one hook call are
/// monotonic, so that keeps each call's events in the order they were made.
pub fn sort_key(e: &Envelope) -> (SystemTime, String) {
    (event_time(e), e.id.clone())
}

pub fn reduce(mut events: Vec<Envelope>, opts: &Options) -> Graph {
    events.sort_by_cached_key(sort_key);
    let mut reducer = Reducer::default();
    for event in &events {
        reducer.apply(event);
    }
    reducer.finish(opts)
}

#[derive(Default)]
struct Reducer {
    nodes: BTreeMap<String, Node>,
    /// Which session each agent process (`<pid>@<start>`) belongs to: the
    /// latest one started in it.
    processes: BTreeMap<String, String>,
    /// The nodes that may have an open wait on each node, so a node's end
    /// closes them without looking at every node.
    waiting_on: BTreeMap<String, BTreeSet<String>>,
    /// The nodes that made each spawn request (by call id; normally one).
    requesters: BTreeMap<String, Vec<String>>,
}

/// Node `session` and its agents (`session/…`): a range of the map, not a
/// look at every node.
fn family<'a>(
    nodes: &'a BTreeMap<String, Node>,
    session: &str,
) -> impl Iterator<Item = (&'a String, &'a Node)> + 'a {
    let prefix = format!("{session}/");
    nodes.get_key_value(session).into_iter().chain(
        nodes
            .range(prefix.clone()..)
            .take_while(move |(id, _)| id.starts_with(&prefix)),
    )
}

impl Reducer {
    fn apply(&mut self, e: &Envelope) {
        let provider = e.source.as_ref().map(|s| s.provider.clone());
        self.ensure(&e.node, provider.as_deref(), &e.ts);
        // An explicit parent wins, unless a spawn binding already settled it.
        if let Some(parent) = &e.parent {
            if self.nodes[&e.node].spawned_by.is_none() {
                self.reparent(&e.node, parent, provider.as_deref(), &e.ts);
            }
        }

        let payload = e.payload();
        let node = self.nodes.get_mut(&e.node).expect("ensured above");
        node.last_event_at = e.ts.clone();
        // Any activity other than a status report means the human answered.
        if node.state == State::InputRequired && !matches!(payload, Payload::Status(_)) {
            node.state = State::Working;
            node.attention = None;
        }

        match payload {
            Payload::SessionStarted(d) => {
                node.cwd = d.cwd.or(node.cwd.take());
                node.title = d.title.or(node.title.take());
                // A resumed session comes back to life. A mid-turn restart
                // (e.g. after compaction) leaves the current state alone.
                if node.state.is_terminal() {
                    node.state = State::Idle;
                    node.ended_at = None;
                }
                // Only if the parent was taken (it's refused if it would
                // put the session under itself).
                if e.parent.is_some() && node.parent == e.parent {
                    node.link = d.link_method.clone().or(Some("env".to_string()));
                }
                let id = e.node.clone();
                if let Some(process) = d.process {
                    node.process = Some(process.clone());
                    // Found by process, a parent must have started first.
                    if e.parent.is_none() && node.parent.is_none() {
                        self.link_by_process(&id, &d.ancestors, &e.ts);
                    }
                    self.processes.insert(process, id.clone());
                } else if e.parent.is_none() && node.parent.is_none() {
                    self.link_by_process(&id, &d.ancestors, &e.ts);
                }
                if self.nodes[&id].parent.is_some() && self.nodes[&id].spawned_by.is_none() {
                    self.bind_session_by_guess(&id);
                }
            }
            Payload::SessionEnded(_) => {
                // A session that already said how it ended (`agent-graph run`
                // reports a failure first) keeps that.
                if !node.state.is_terminal() {
                    node.state = State::Completed;
                }
                node.ended_at = Some(e.ts.clone());
                let id = e.node.clone();
                self.close_waits_on(&id, &e.ts, true);
                self.cancel_unfinished_descendants(&id, &e.ts);
                self.calls_over(&id, &e.ts);
            }
            Payload::AgentSpawned(d) => {
                node.agent_type = d.agent_type.or(node.agent_type.take());
                node.purpose = d.purpose.or(node.purpose.take());
                node.background = d.background.or(node.background);
                if !node.state.is_terminal() {
                    node.state = State::Working;
                }
                if node.spawned_by.is_none() {
                    self.bind_by_guess(&e.node);
                }
            }
            Payload::AgentFinished(d) => {
                node.state = match d.status {
                    FinishStatus::Completed => State::Completed,
                    FinishStatus::Failed => State::Failed,
                    FinishStatus::Canceled => State::Canceled,
                };
                node.summary = d.summary.or(node.summary.take());
                node.ended_at = Some(e.ts.clone());
                let id = e.node.clone();
                // Not spawn waits: which request this agent answers may still
                // be a guess, and `spawn.returned` will close the right one.
                self.close_waits_on(&id, &e.ts, false);
                self.calls_over(&id, &e.ts);
            }
            Payload::Status(d) => {
                if d.state == State::Idle || d.state.is_terminal() {
                    self.calls_over(&e.node, &e.ts);
                }
                let node = self.nodes.get_mut(&e.node).expect("ensured above");
                node.state = d.state;
                if d.state == State::InputRequired {
                    node.attention = d.summary;
                } else {
                    node.attention = None;
                    node.summary = d.summary.or(node.summary.take());
                }
                if d.state.is_terminal() {
                    node.ended_at = Some(e.ts.clone());
                } else {
                    node.ended_at = None;
                }
            }
            Payload::TasksUpdated(d) => {
                node.tasks = d
                    .items
                    .into_iter()
                    .map(|t| Task {
                        id: t.id,
                        text: t.text,
                        active_text: t.active_text,
                        status: t.status,
                    })
                    .collect();
            }
            Payload::TaskUpserted(d) => match node.tasks.iter_mut().find(|t| t.id == d.id) {
                Some(task) => {
                    if let Some(text) = d.text {
                        task.text = text;
                    }
                    task.active_text = d.active_text.or(task.active_text.take());
                    if let Some(status) = d.status {
                        task.status = status;
                    }
                }
                None => node.tasks.push(Task {
                    text: d.text.unwrap_or_else(|| format!("Task {}", d.id)),
                    id: d.id,
                    active_text: d.active_text,
                    status: d.status.unwrap_or(TaskStatus::Pending),
                }),
            },
            Payload::TaskDeleted(d) => node.tasks.retain(|t| t.id != d.id),
            Payload::SpawnRequested(d) => {
                if node.spawns.iter().any(|s| s.call_id == d.call_id) {
                    return;
                }
                if !d.background {
                    node.waits.push(Wait {
                        wait_id: d.call_id.clone(),
                        on: None,
                        reason: d.purpose.clone(),
                        open: true,
                        spawn: true,
                        started_at: e.ts.clone(),
                        ended_at: None,
                    });
                }
                self.requesters
                    .entry(d.call_id.clone())
                    .or_default()
                    .push(e.node.clone());
                node.spawns.push(Spawn {
                    call_id: d.call_id,
                    kind: d.kind,
                    agent_type: d.agent_type,
                    purpose: d.purpose,
                    background: d.background,
                    child: None,
                    returned: false,
                    requested_at: e.ts.clone(),
                });
            }
            Payload::SpawnReturned(d) => {
                end_wait(node, &d.call_id, &e.ts);
                let requester = e.node.clone();
                let known = match node.spawns.iter_mut().find(|s| s.call_id == d.call_id) {
                    Some(spawn) => {
                        spawn.returned = true;
                        true
                    }
                    None => false,
                };
                if let Some(child) = d.child {
                    self.ensure(&child, provider.as_deref(), &e.ts);
                    if known {
                        self.bind(&requester, &d.call_id, &child);
                    } else {
                        self.reparent(&child, &requester, provider.as_deref(), &e.ts);
                    }
                }
            }
            Payload::WaitStarted(d) => {
                if !node.waits.iter().any(|w| w.wait_id == d.wait_id && w.open) {
                    self.waiting_on
                        .entry(d.on.clone())
                        .or_default()
                        .insert(e.node.clone());
                    node.waits.push(Wait {
                        wait_id: d.wait_id,
                        on: Some(d.on),
                        reason: d.reason,
                        open: true,
                        spawn: false,
                        started_at: e.ts.clone(),
                        ended_at: None,
                    });
                }
            }
            Payload::WaitEnded(d) => end_wait(node, &d.wait_id, &e.ts),
            Payload::MessageSent(d) => {
                // The same message recorded twice (say, hooks installed in
                // two settings files with different commands) counts once.
                let seen = node
                    .messages
                    .iter()
                    .any(|m| m.direction == Direction::Sent && m.message_id == d.message_id);
                if seen {
                    return;
                }
                let target = self.resolve(&d.to, &e.node);
                let sender = self.nodes.get_mut(&e.node).expect("ensured above");
                sender.messages.push(Message {
                    message_id: d.message_id.clone(),
                    direction: Direction::Sent,
                    peer: target.clone().unwrap_or_else(|| d.to.clone()),
                    reply_to: d.reply_to.clone(),
                    summary: d.summary.clone(),
                    body: d.body.clone(),
                    ts: e.ts.clone(),
                });
                if let Some(target) = target.and_then(|t| self.nodes.get_mut(&t)) {
                    target.messages.push(Message {
                        message_id: d.message_id,
                        direction: Direction::Received,
                        peer: e.node.clone(),
                        reply_to: d.reply_to,
                        summary: d.summary,
                        body: d.body,
                        ts: e.ts.clone(),
                    });
                }
            }
            Payload::Activity(_) | Payload::Unknown(_) => {}
        }
    }

    /// Creates a placeholder for `id` (and its parent, for agent ids) if needed.
    fn ensure(&mut self, id: &str, provider: Option<&str>, ts: &str) {
        if self.nodes.contains_key(id) {
            return;
        }
        let (kind, parent) = match id.rsplit_once('/') {
            Some((parent, _)) => (NodeKind::Agent, Some(parent.to_string())),
            None => (NodeKind::Session, None),
        };
        // Node ids start with their provider, which is right even for a
        // placeholder made from another provider's event.
        let provider = id
            .split_once(':')
            .map(|(p, _)| p.to_string())
            .or_else(|| provider.map(String::from))
            .unwrap_or_default();
        self.nodes.insert(
            id.to_string(),
            Node {
                id: id.to_string(),
                kind,
                provider: provider.clone(),
                parent: None,
                children: Vec::new(),
                state: if kind == NodeKind::Session {
                    State::Idle
                } else {
                    State::Working
                },
                summary: None,
                attention: None,
                title: None,
                cwd: None,
                agent_type: None,
                purpose: None,
                background: None,
                spawned_by: None,
                link: None,
                process: None,
                tasks: Vec::new(),
                spawns: Vec::new(),
                waits: Vec::new(),
                messages: Vec::new(),
                started_at: Some(ts.to_string()),
                ended_at: None,
                last_event_at: ts.to_string(),
                headline: None,
                open_tasks: 0,
                blocked: None,
                stale: false,
            },
        );
        if let Some(parent) = parent {
            self.reparent(id, &parent, Some(&provider), ts);
        }
    }

    /// Moves `child` under `parent`, unless that would put it under itself.
    /// Every change of parent comes through here (explicit parents, spawn
    /// bindings, a subagent's inferred session), so the graph never has a
    /// cycle: a session resumed from its own child's shell, say, stays on top.
    fn reparent(&mut self, child: &str, parent: &str, provider: Option<&str>, ts: &str) {
        if child == parent || self.nodes[child].parent.as_deref() == Some(parent) {
            return;
        }
        self.ensure(parent, provider, ts);
        if self.is_under(parent, child) {
            return;
        }
        if let Some(old) = self.nodes[child].parent.clone() {
            if let Some(old) = self.nodes.get_mut(&old) {
                old.children.retain(|c| c != child);
            }
        }
        self.nodes.get_mut(child).expect("exists").parent = Some(parent.to_string());
        let parent = self.nodes.get_mut(parent).expect("ensured");
        if !parent.children.iter().any(|c| c == child) {
            parent.children.push(child.to_string());
        }
    }

    /// Binds spawn `call_id` on `requester` to `child`, undoing any earlier guess.
    fn bind(&mut self, requester: &str, call_id: &str, child: &str) {
        // A child that's above its requester can't have been started by it.
        if self.is_under(requester, child) {
            return;
        }
        // Undo a guess that bound this spawn to a different child.
        let previous = self.nodes[requester]
            .spawns
            .iter()
            .find(|s| s.call_id == call_id)
            .and_then(|s| s.child.clone());
        let displaced = previous.filter(|p| p != child);
        if let Some(previous) = &displaced {
            if let Some(n) = self.nodes.get_mut(previous) {
                n.spawned_by = None;
            }
        }
        // Undo a guess that bound this child to a different spawn.
        if let Some(other_call) = self.nodes[child]
            .spawned_by
            .clone()
            .filter(|c| c != call_id)
        {
            for id in self.requesters.get(&other_call).into_iter().flatten() {
                let Some(node) = self.nodes.get_mut(id) else {
                    continue;
                };
                // Only the request it was bound to: call ids aren't unique
                // across sessions, and another's may have a child of its own.
                if let Some(spawn) = node
                    .spawns
                    .iter_mut()
                    .find(|s| s.call_id == other_call && s.child.as_deref() == Some(child))
                {
                    spawn.child = None;
                    if let Some(wait) = node.waits.iter_mut().find(|w| w.wait_id == other_call) {
                        wait.on = None;
                    }
                }
            }
        }

        let node = self.nodes.get_mut(requester).expect("exists");
        let Some(spawn) = node.spawns.iter_mut().find(|s| s.call_id == call_id) else {
            return;
        };
        spawn.child = Some(child.to_string());
        let (purpose, agent_type, background) = (
            spawn.purpose.clone(),
            spawn.agent_type.clone(),
            spawn.background,
        );
        if let Some(wait) = node.waits.iter_mut().find(|w| w.wait_id == call_id) {
            wait.on = Some(child.to_string());
            self.waiting_on
                .entry(child.to_string())
                .or_default()
                .insert(requester.to_string());
        }

        let child_node = self.nodes.get_mut(child).expect("exists");
        child_node.spawned_by = Some(call_id.to_string());
        // The request is where the purpose comes from, so it also replaces one
        // left behind by an earlier wrong guess.
        if purpose.is_some() {
            child_node.purpose = purpose;
        }
        child_node.agent_type = child_node.agent_type.take().or(agent_type);
        child_node.background = Some(background);
        let ts = child_node.last_event_at.clone();
        self.reparent(child, requester, None, &ts);

        // The child a wrong guess had put here still came from some request;
        // pair it with whichever one is left.
        if let Some(previous) = displaced {
            if self
                .nodes
                .get(&previous)
                .is_some_and(|n| n.spawned_by.is_none())
            {
                self.bind_by_guess(&previous);
            }
        }
    }

    /// Guesses which spawn request started `child`, for providers (and hook
    /// orderings) where the start event arrives before we know for sure. It
    /// picks the oldest unbound request in the same session, preferring one
    /// with the same agent type. A later `spawn.returned` corrects it.
    fn bind_by_guess(&mut self, child: &str) {
        let session = child.split('/').next().unwrap_or(child);
        let child_type = self.nodes[child].agent_type.clone();
        let mut candidates: Vec<(String, String, String, Option<String>)> =
            family(&self.nodes, session)
                .flat_map(|(id, n)| {
                    n.spawns
                        .iter()
                        .filter(|s| s.child.is_none() && s.kind == SpawnKind::Agent && !s.returned)
                        .map(move |s| {
                            (
                                s.requested_at.clone(),
                                id.clone(),
                                s.call_id.clone(),
                                s.agent_type.clone(),
                            )
                        })
                })
                .collect();
        candidates.sort();
        let pick = candidates
            .iter()
            .find(|c| c.3.is_some() && c.3 == child_type)
            .or_else(|| candidates.iter().find(|c| c.3.is_none()))
            .or_else(|| {
                if candidates.len() == 1 {
                    candidates.first()
                } else {
                    None
                }
            });
        if let Some((_, requester, call_id, _)) = pick.cloned() {
            self.bind(&requester, &call_id, child);
        }
    }

    /// Links session `id` to the session whose agent process is the nearest
    /// of `ancestors` (the processes above its own agent's).
    fn link_by_process(&mut self, id: &str, ancestors: &[String], ts: &str) {
        let parent = ancestors
            .iter()
            .find_map(|p| self.processes.get(p))
            .filter(|p| *p != id && !self.is_under(p, id))
            .cloned();
        if let Some(parent) = parent {
            self.reparent(id, &parent, None, ts);
            self.nodes.get_mut(id).expect("exists").link = Some("process".to_string());
        }
    }

    /// Whether `node` is `ancestor` or somewhere below it.
    fn is_under(&self, node: &str, ancestor: &str) -> bool {
        let mut next = Some(node.to_string());
        let mut steps = 0;
        while let Some(id) = next {
            if id == ancestor {
                return true;
            }
            steps += 1;
            if steps > self.nodes.len() {
                return false;
            }
            next = self.nodes.get(&id).and_then(|n| n.parent.clone());
        }
        false
    }

    /// Pairs a session that has just linked itself to a parent with the
    /// request that started it: the oldest unpaired `kind: session` request
    /// in the parent's session (or its agents) made before it started,
    /// preferring one for the same program. Nothing names the child when the
    /// shell command returns, so this guess is final.
    fn bind_session_by_guess(&mut self, child: &str) {
        let Some(parent) = self.nodes[child].parent.clone() else {
            return;
        };
        let parent_session = parent.split('/').next().unwrap_or(&parent).to_string();
        let started = self.nodes[child].started_at.clone().unwrap_or_default();
        let mut candidates: Vec<(String, String, String, Option<String>)> =
            family(&self.nodes, &parent_session)
                .flat_map(|(id, n)| {
                    n.spawns
                        .iter()
                        .filter(|s| {
                            // A background launch returns before its child
                            // starts; a foreground one only once it's done.
                            s.kind == SpawnKind::Session
                                && s.child.is_none()
                                && (s.background || !s.returned)
                                && s.requested_at <= started
                        })
                        .map(move |s| {
                            (
                                s.requested_at.clone(),
                                id.clone(),
                                s.call_id.clone(),
                                s.agent_type.clone(),
                            )
                        })
                })
                .collect();
        candidates.sort();
        let child_node = &self.nodes[child];
        let pick = candidates
            .iter()
            .find(|c| c.3.as_deref().is_some_and(|p| runs(child_node, p)))
            .or_else(|| candidates.first());
        if let Some((_, requester, call_id, _)) = pick.cloned() {
            self.bind(&requester, &call_id, child);
        }
    }

    /// `id`'s turn is over (it's idle) or it has ended, so every call it made
    /// to start a child in the foreground has returned, whether or not the
    /// provider said so: Claude Code only reports calls that succeed. Their
    /// waits close, and they can't be paired with a later child.
    fn calls_over(&mut self, id: &str, ts: &str) {
        let Some(node) = self.nodes.get_mut(id) else {
            return;
        };
        for wait in node.waits.iter_mut().filter(|w| w.open && w.spawn) {
            wait.open = false;
            wait.ended_at = Some(ts.to_string());
        }
        for spawn in node.spawns.iter_mut().filter(|s| !s.background) {
            spawn.returned = true;
        }
    }

    /// Closes the open waits on `target` (its spawn waits too, if asked).
    fn close_waits_on(&mut self, target: &str, ts: &str, spawn_waits_too: bool) {
        let Some(waiters) = self.waiting_on.remove(target) else {
            return;
        };
        let on_target = |w: &Wait| w.open && w.on.as_deref() == Some(target);
        let mut still = BTreeSet::new();
        for id in waiters {
            let Some(node) = self.nodes.get_mut(&id) else {
                continue;
            };
            for wait in node
                .waits
                .iter_mut()
                .filter(|w| on_target(w) && (spawn_waits_too || !w.spawn))
            {
                wait.open = false;
                wait.ended_at = Some(ts.to_string());
            }
            if node.waits.iter().any(on_target) {
                still.insert(id);
            }
        }
        if !still.is_empty() {
            self.waiting_on.insert(target.to_string(), still);
        }
    }

    /// A session's agents stop with it. Sessions it started are processes of
    /// their own, which report their own ends, so they (and what's under
    /// them) are left alone.
    fn cancel_unfinished_descendants(&mut self, id: &str, ts: &str) {
        let mut queue: VecDeque<String> = self.nodes[id].children.iter().cloned().collect();
        let mut seen = BTreeSet::new();
        while let Some(child) = queue.pop_front() {
            if !seen.insert(child.clone()) {
                continue;
            }
            let Some(node) = self.nodes.get_mut(&child) else {
                continue;
            };
            if node.kind == NodeKind::Session {
                continue;
            }
            if !node.state.is_terminal() {
                node.state = State::Canceled;
                node.ended_at = Some(ts.to_string());
            }
            queue.extend(node.children.iter().cloned());
            self.calls_over(&child, ts);
        }
    }

    /// Resolves a message recipient to a node id: an exact node id, or an
    /// agent id within the sender's session.
    fn resolve(&self, to: &str, sender: &str) -> Option<String> {
        if self.nodes.contains_key(to) {
            return Some(to.to_string());
        }
        let session = sender.split('/').next().unwrap_or(sender);
        let candidate = format!("{session}/{to}");
        self.nodes.contains_key(&candidate).then_some(candidate)
    }

    fn finish(mut self, opts: &Options) -> Graph {
        // A wait whose target has finished isn't holding anything up, even if
        // the event that formally ends it never arrived.
        let finished: BTreeSet<String> = self
            .nodes
            .values()
            .filter(|n| n.state.is_terminal())
            .map(|n| n.id.clone())
            .collect();
        let active = |w: &Wait| w.open && w.on.as_ref().is_none_or(|t| !finished.contains(t));

        for node in self.nodes.values_mut() {
            node.open_tasks = node.tasks.iter().filter(|t| t.status.is_open()).count();
            node.headline = headline(node);
            let quiet_for = humantime::parse_rfc3339_weak(&node.last_event_at)
                .ok()
                .and_then(|t| opts.now.duration_since(t).ok())
                .unwrap_or_default();
            // Only a node that should be making progress can be hung. Idle and
            // input_required nodes are legitimately quiet, and a blocked node
            // is waiting on someone else (who may be the stale one).
            let waiting = node.waits.iter().any(active);
            node.stale = node.state == State::Working && !waiting && quiet_for > opts.stale_after;
        }

        // A node that has finished isn't waiting on anything.
        let blocked: Vec<(String, Blocked)> = self
            .nodes
            .values()
            .filter(|n| !n.state.is_terminal() && n.waits.iter().any(active))
            .map(|n| (n.id.clone(), self.blocked(n, &active)))
            .collect();
        for (id, b) in blocked {
            self.nodes.get_mut(&id).expect("exists").blocked = Some(b);
        }

        for id in self.nodes.keys().cloned().collect::<Vec<_>>() {
            let mut children = self.nodes[&id].children.clone();
            children.sort_by_key(|c| self.nodes.get(c).and_then(|n| n.started_at.clone()));
            self.nodes.get_mut(&id).expect("exists").children = children;
        }
        let mut roots: Vec<&Node> = self
            .nodes
            .values()
            .filter(|n| {
                n.parent
                    .as_ref()
                    .is_none_or(|p| !self.nodes.contains_key(p))
            })
            .collect();
        roots.sort_by(|a, b| b.last_event_at.cmp(&a.last_event_at));
        let roots = roots.into_iter().map(|n| n.id.clone()).collect();

        Graph {
            nodes: self.nodes,
            roots,
        }
    }

    fn blocked(&self, node: &Node, active: &dyn Fn(&Wait) -> bool) -> Blocked {
        let open: Vec<&Wait> = node.waits.iter().filter(|w| active(w)).collect();
        let on: Vec<String> = open.iter().filter_map(|w| w.on.clone()).collect();
        let starting = open.iter().filter(|w| w.on.is_none()).count();

        let mut seen = BTreeSet::new();
        let mut cycle = false;
        let mut queue: VecDeque<String> = on.iter().cloned().collect();
        while let Some(id) = queue.pop_front() {
            if id == node.id {
                cycle = true;
                continue;
            }
            if !seen.insert(id.clone()) {
                continue;
            }
            let Some(n) = self.nodes.get(&id) else {
                continue;
            };
            queue.extend(n.children.iter().cloned());
            // A node that has finished isn't waiting on anything.
            if !n.state.is_terminal() {
                queue.extend(
                    n.waits
                        .iter()
                        .filter(|w| active(w))
                        .filter_map(|w| w.on.clone()),
                );
            }
        }
        let reached: Vec<&Node> = seen.iter().filter_map(|id| self.nodes.get(id)).collect();
        Blocked {
            on,
            starting,
            nodes: reached.iter().filter(|n| !n.state.is_terminal()).count(),
            open_tasks: reached
                .iter()
                .map(|n| n.tasks.iter().filter(|t| t.status.is_open()).count())
                .sum(),
            cycle,
        }
    }
}

/// Whether session `node` is (probably) running `program`, the command a
/// spawn request named: `claude` for a `claude-code` session, or the program
/// `agent-graph run` was given.
fn runs(node: &Node, program: &str) -> bool {
    node.provider == program
        || node.provider.starts_with(&format!("{program}-"))
        || (node.provider == "run" && node.title.as_deref() == Some(program))
}

fn end_wait(node: &mut Node, wait_id: &str, ts: &str) {
    for wait in node
        .waits
        .iter_mut()
        .filter(|w| w.open && w.wait_id == wait_id)
    {
        wait.open = false;
        wait.ended_at = Some(ts.to_string());
    }
}

fn headline(node: &Node) -> Option<String> {
    if let Some(summary) = &node.summary {
        return Some(summary.clone());
    }
    let current = node
        .tasks
        .iter()
        .find(|t| t.status == TaskStatus::InProgress);
    let pending = node
        .tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Pending)
        .count();
    match current {
        Some(task) => {
            let text = task
                .active_text
                .clone()
                .unwrap_or_else(|| task.text.clone());
            Some(if pending > 0 {
                format!("{text} (+{pending} pending)")
            } else {
                text
            })
        }
        None if pending == 1 => Some("1 task pending".to_string()),
        None if pending > 1 => Some(format!("{pending} tasks pending")),
        None => None,
    }
}
