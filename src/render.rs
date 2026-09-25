//! Text rendering of the graph, for `agent-graph tree` (plain) and
//! `agent-graph tail` (coloured, live).
//!
//! Lines are built from spans that carry a tone, so the same layout can be
//! printed plain or styled.

use std::path::Path;

use crate::event::State;
use crate::reducer::{Graph, Node, NodeKind};

/// How a span should look; the terminal view maps these to colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Plain,
    Dim,
    Strong,
    Working,
    Attention,
    Good,
    Bad,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    pub text: String,
    pub tone: Tone,
}

pub type Line = Vec<Span>;

fn span(text: impl Into<String>, tone: Tone) -> Span {
    Span {
        text: clean(&text.into()),
        tone,
    }
}

/// Makes model-written text safe to print to a terminal. Control characters
/// (newlines, and escape sequences that could recolour, clear or rewrite the
/// screen) become spaces; bidirectional overrides, which can make text read
/// differently from what it is, are dropped.
pub fn clean(text: &str) -> String {
    text.chars()
        .filter(|c| !matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'))
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// Characters for the tree's branches.
#[derive(Debug, Clone, Copy)]
pub struct Branches {
    tee: &'static str,
    last: &'static str,
    pipe: &'static str,
    gap: &'static str,
}

pub const UNICODE: Branches = Branches {
    tee: "├─ ",
    last: "└─ ",
    pipe: "│  ",
    gap: "   ",
};
pub const ASCII: Branches = Branches {
    tee: "|- ",
    last: "`- ",
    pipe: "|  ",
    gap: "   ",
};

/// The trees under `roots` as plain text, one node per line.
pub fn tree(graph: &Graph, roots: &[String]) -> String {
    lines(graph, roots, UNICODE)
        .iter()
        .map(|line| line.iter().map(|s| s.text.as_str()).collect::<String>() + "\n")
        .collect()
}

/// The trees under `roots` as styled lines.
pub fn lines(graph: &Graph, roots: &[String], branches: Branches) -> Vec<Line> {
    let mut out = Vec::new();
    for root in roots {
        if let Some(node) = graph.nodes.get(root) {
            out.push(node_line(graph, node));
            children(graph, node, "", branches, &mut out);
        }
    }
    out
}

fn children(graph: &Graph, node: &Node, prefix: &str, b: Branches, out: &mut Vec<Line>) {
    let kids: Vec<&Node> = node
        .children
        .iter()
        .filter_map(|c| graph.nodes.get(c))
        .collect();
    for (i, child) in kids.iter().enumerate() {
        let last = i + 1 == kids.len();
        let mut line = vec![span(
            format!("{prefix}{}", if last { b.last } else { b.tee }),
            Tone::Dim,
        )];
        line.extend(node_line(graph, child));
        out.push(line);
        let next = format!("{prefix}{}", if last { b.gap } else { b.pipe });
        children(graph, child, &next, b, out);
    }
}

fn node_line(graph: &Graph, node: &Node) -> Line {
    let local_id = node.id.rsplit(['/', ':']).next().unwrap_or(&node.id);
    let mut parts: Vec<Vec<Span>> = Vec::new();

    parts.push(match node.kind {
        NodeKind::Session => {
            // Named by its title (e.g. `agent-graph run --name`), else its folder.
            let dir = node.title.as_deref().map(Into::into).or_else(|| {
                node.cwd
                    .as_deref()
                    .and_then(|c| Path::new(c).file_name())
                    .map(|f| f.to_string_lossy())
            });
            let id = span(format!("{}:{}", node.provider, short(local_id)), Tone::Dim);
            match dir {
                Some(dir) => vec![id, span("  ", Tone::Plain), span(dir, Tone::Strong)],
                None => vec![id],
            }
        }
        NodeKind::Agent => vec![span(
            format!(
                "{} {}",
                node.agent_type.as_deref().unwrap_or("agent"),
                short(local_id)
            ),
            Tone::Strong,
        )],
    });

    let mut state = vec![span(
        format!("[{}]", node.state.as_str()),
        state_tone(node.state),
    )];
    if node.stale {
        state.push(span(" (stale?)", Tone::Bad));
    }
    parts.push(state);

    if let Some(purpose) = &node.purpose {
        parts.push(vec![span(purpose.clone(), Tone::Plain)]);
    }
    if node.state == State::InputRequired {
        let ask = node.attention.as_deref().unwrap_or("waiting for input");
        parts.push(vec![span(format!("needs you: {ask}"), Tone::Attention)]);
    } else if let Some(headline) = &node.headline {
        parts.push(vec![span(headline.clone(), Tone::Dim)]);
    }
    if !node.tasks.is_empty() {
        let done = node.tasks.len() - node.open_tasks;
        parts.push(vec![span(
            format!("tasks {done}/{}", node.tasks.len()),
            Tone::Dim,
        )]);
    }
    if let Some(blocked) = &node.blocked {
        let mut waiting = Vec::new();
        if blocked.cycle {
            waiting.push(span("DEADLOCK", Tone::Bad));
        }
        if !blocked.on.is_empty() {
            let n = blocked.nodes.max(blocked.on.len());
            waiting.push(span(
                format!(
                    "waiting on {n} node{} ({} open task{})",
                    plural(n),
                    blocked.open_tasks,
                    plural(blocked.open_tasks)
                ),
                Tone::Working,
            ));
            // Name a stale target: that's usually why the wait is long.
            if let Some(stuck) = blocked
                .on
                .iter()
                .filter_map(|id| graph.nodes.get(id))
                .find(|n| n.stale)
            {
                waiting.push(span(format!("{} looks stuck", name(stuck)), Tone::Bad));
            }
        }
        if blocked.starting > 0 {
            waiting.push(span(
                format!("{} starting", blocked.starting),
                Tone::Working,
            ));
        }
        let mut joined = Vec::new();
        for (i, s) in waiting.into_iter().enumerate() {
            if i > 0 {
                joined.push(span(", ", Tone::Plain));
            }
            joined.push(s);
        }
        parts.push(joined);
    }

    let mut line = Vec::new();
    for (i, part) in parts.into_iter().enumerate() {
        if i > 0 {
            line.push(span("  ", Tone::Plain));
        }
        line.extend(part);
    }
    line
}

pub fn state_tone(state: State) -> Tone {
    match state {
        State::Working => Tone::Working,
        State::InputRequired => Tone::Attention,
        State::Idle | State::Canceled => Tone::Dim,
        State::Completed => Tone::Good,
        State::Failed => Tone::Bad,
    }
}

/// A short name for an agent or session, as the tree shows it.
pub fn name(node: &Node) -> String {
    let local = node.id.rsplit(['/', ':']).next().unwrap_or(&node.id);
    match node.kind {
        NodeKind::Session => node
            .title
            .clone()
            .or_else(|| {
                node.cwd
                    .as_deref()
                    .and_then(|c| Path::new(c).file_name())
                    .map(|f| f.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| format!("session {}", short(local))),
        NodeKind::Agent => format!(
            "{} {}",
            node.agent_type.as_deref().unwrap_or("agent"),
            short(local)
        ),
    }
}

/// What a card calls its node: an agent's type and id; a session's title
/// (`agent-graph run --name`); a session started by another, its agent and
/// id (its folder is often its parent's); otherwise just "Session".
pub fn card_name(node: &Node) -> String {
    match (node.kind, &node.title, &node.parent) {
        (NodeKind::Agent, _, _) => name(node),
        (NodeKind::Session, Some(title), _) => title.clone(),
        (NodeKind::Session, None, Some(_)) => {
            let local = node.id.rsplit(['/', ':']).next().unwrap_or(&node.id);
            format!("{} session {}", provider_name(&node.provider), short(local))
        }
        (NodeKind::Session, None, None) => "Session".to_string(),
    }
}

/// A provider's name as people know it.
pub fn provider_name(provider: &str) -> &str {
    match provider {
        "claude-code" => "Claude Code",
        "codex" => "Codex",
        "gemini" => "Gemini CLI",
        "cursor" => "Cursor",
        other => other,
    }
}

fn short(id: &str) -> &str {
    id.char_indices().nth(8).map_or(id, |(i, _)| &id[..i])
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}
