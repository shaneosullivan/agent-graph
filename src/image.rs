//! Renders the graph as an image sized for a phone screen: SVG first, then
//! PNG through `resvg` (pure Rust, so it works the same on every platform).
//!
//! Event text comes from models, so everything written into the SVG is escaped.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::sync::{Arc, OnceLock};
use std::time::SystemTime;

use resvg::{tiny_skia, usvg};

use crate::event::{State, TaskStatus};
use crate::reducer::{Blocked, Graph, Node, NodeKind};

/// Logical width; the PNG is rendered at `SCALE` times this.
const WIDTH: f32 = 540.0;
const SCALE: f32 = 2.0;
const PAD: f32 = 20.0;
const INDENT: f32 = 22.0;
const FONT: &str = "Inter, -apple-system, 'Segoe UI', 'Helvetica Neue', Helvetica, Arial, \
                    'Noto Sans', 'DejaVu Sans', 'Liberation Sans', sans-serif";
const MAX_TASKS: usize = 12;
/// Agents drawn per session, so a huge one still makes a picture that
/// renders and fits on a phone; the rest are counted.
const MAX_AGENTS: usize = 30;
const MAX_CALLOUTS: usize = 3;
/// The most a card is indented (six full steps), so cards keep room for
/// their text however deep the tree.
const MAX_INDENT: f32 = 6.0 * INDENT;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    #[default]
    Light,
    Dark,
}

struct Palette {
    bg: &'static str,
    panel: &'static str,
    panel_done: &'static str,
    text: &'static str,
    muted: &'static str,
    faint: &'static str,
    line: &'static str,
    working: &'static str,
    input: &'static str,
    input_soft: &'static str,
    idle: &'static str,
    completed: &'static str,
    failed: &'static str,
}

const LIGHT: Palette = Palette {
    bg: "#f6f6f3",
    panel: "#ffffff",
    panel_done: "#fafaf8",
    text: "#1c1c1e",
    muted: "#6b6b72",
    faint: "#9c9ca3",
    line: "#e5e4df",
    working: "#2f6fed",
    input: "#c26a00",
    input_soft: "#fff4e2",
    idle: "#8a8a91",
    completed: "#1f9d55",
    failed: "#d93025",
};

const DARK: Palette = Palette {
    bg: "#131315",
    panel: "#1b1b1e",
    panel_done: "#18181b",
    text: "#ececef",
    muted: "#a0a0a8",
    faint: "#75757d",
    line: "#2b2b30",
    working: "#6b9bff",
    input: "#f0a33a",
    input_soft: "#33260f",
    idle: "#8d8d96",
    completed: "#45c47c",
    failed: "#ff6b5e",
};

pub struct Options {
    pub theme: Theme,
    /// The moment the graph shows, printed in the header.
    pub as_of: SystemTime,
}

/// The SVG for the trees under `roots`.
pub fn svg(graph: &Graph, roots: &[String], opts: &Options) -> String {
    let p = match opts.theme {
        Theme::Light => &LIGHT,
        Theme::Dark => &DARK,
    };
    let mut c = Canvas::default();
    let mut y = PAD;

    for (i, root) in roots.iter().enumerate() {
        let Some(node) = graph.nodes.get(root) else {
            continue;
        };
        if i > 0 {
            y += 8.0;
            c.line(PAD, y, WIDTH - PAD, y, p.line, 1.0);
            y += 22.0;
        }
        y = session(&mut c, graph, node, y, p);
    }

    y += 6.0;
    let stamp = humantime::format_rfc3339_seconds(opts.as_of).to_string();
    let stamp = stamp.trim_end_matches('Z').replace('T', " ");
    c.text(PAD, y + 12.0, 11.0, 400, p.faint, "start", "agent-graph");
    c.text(
        WIDTH - PAD,
        y + 12.0,
        11.0,
        400,
        p.faint,
        "end",
        &format!("as of {stamp} UTC"),
    );
    y += 12.0 + PAD;

    let height = y.ceil();
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{WIDTH}\" height=\"{height}\" \
         viewBox=\"0 0 {WIDTH} {height}\" font-family=\"{FONT}\">\
         <rect width=\"100%\" height=\"100%\" fill=\"{}\"/>{}</svg>",
        p.bg, c.out
    )
}

/// Rasterizes `svg` to PNG bytes at twice its size, for sharp phone screens.
pub fn png(svg: &str) -> Result<Vec<u8>, String> {
    let mut opts = usvg::Options {
        fontdb: fonts(),
        ..Default::default()
    };
    opts.font_family = default_family(&opts.fontdb);
    let tree = usvg::Tree::from_str(svg, &opts).map_err(|e| format!("rendering image: {e}"))?;
    let size = tree.size();
    let (w, h) = (
        (size.width() * SCALE).ceil() as u32,
        (size.height() * SCALE).ceil() as u32,
    );
    let mut pixmap = tiny_skia::Pixmap::new(w, h).ok_or("image is too large")?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(SCALE, SCALE),
        &mut pixmap.as_mut(),
    );
    pixmap
        .encode_png()
        .map_err(|e| format!("encoding PNG: {e}"))
}

/// System fonts, loaded once per process.
fn fonts() -> Arc<usvg::fontdb::Database> {
    static DB: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = usvg::fontdb::Database::new();
        db.load_system_fonts();
        let family = default_family(&db);
        db.set_sans_serif_family(family);
        Arc::new(db)
    })
    .clone()
}

/// The first commonly installed sans-serif family this machine has.
fn default_family(db: &usvg::fontdb::Database) -> String {
    const PREFERRED: &[&str] = &[
        "Inter",
        "Helvetica Neue",
        "Helvetica",
        "Segoe UI",
        "Arial",
        "Noto Sans",
        "DejaVu Sans",
        "Liberation Sans",
    ];
    PREFERRED
        .iter()
        .find(|want| {
            db.faces()
                .any(|f| f.families.iter().any(|(name, _)| name == *want))
        })
        .map(|s| s.to_string())
        .or_else(|| {
            db.faces()
                .next()
                .and_then(|f| f.families.first().map(|(n, _)| n.clone()))
        })
        .unwrap_or_else(|| "sans-serif".to_string())
}

/// One session: a header, callouts for anything that needs the user, the
/// tree of agents, and the session's tasks. Returns the next free y.
fn session(c: &mut Canvas, graph: &Graph, root: &Node, mut y: f32, p: &Palette) -> f32 {
    let nodes = subtree(graph, root);
    let agents = nodes.len() - 1;

    // Header.
    let title = match root.kind {
        NodeKind::Session => root
            .title
            .as_deref()
            .or_else(|| {
                root.cwd
                    .as_deref()
                    .and_then(|cwd| cwd.rsplit(['/', '\\']).find(|s| !s.is_empty()))
            })
            .unwrap_or("Session")
            .to_string(),
        NodeKind::Agent => name(root),
    };
    let title = fit(
        &title,
        WIDTH - 2.0 * PAD - 10.0 - pill_width(root.state, root.stale),
        20.0,
        true,
    );
    c.text(PAD, y + 20.0, 20.0, 700, p.text, "start", &title);
    let pill_x = PAD + text_width(&title, 20.0, true) + 10.0;
    c.pill(pill_x, y + 5.0, root.state, root.stale, p);
    y += 32.0;

    let mut meta = vec![root.provider.clone(), short(local(&root.id)).to_string()];
    if agents > 0 {
        meta.push(plural(agents, "agent"));
    }
    if !root.tasks.is_empty() {
        meta.push(format!(
            "tasks {}/{}",
            root.tasks.len() - root.open_tasks,
            root.tasks.len()
        ));
    }
    c.text(
        PAD,
        y + 12.0,
        12.0,
        400,
        p.muted,
        "start",
        &meta.join(" · "),
    );
    y += 26.0;

    // Anything that needs the user, first.
    let waiting: Vec<&&Node> = nodes
        .iter()
        .filter(|n| n.state == State::InputRequired)
        .collect();
    for node in waiting.iter().take(MAX_CALLOUTS) {
        let who = if node.kind == NodeKind::Session {
            crate::render::card_name(node)
        } else {
            name(node)
        };
        let msg = format!(
            "{who} needs you: {}",
            node.attention.as_deref().unwrap_or("waiting for input")
        );
        let lines = wrap(&msg, WIDTH - 2.0 * PAD - 24.0, 13.0, true, 3);
        let h = 16.0 + lines.len() as f32 * 18.0;
        c.rect(PAD, y, WIDTH - 2.0 * PAD, h, 8.0, p.input_soft, None);
        c.rect(PAD, y, 3.0, h, 1.5, p.input, None);
        for (i, line) in lines.iter().enumerate() {
            c.text(
                PAD + 14.0,
                y + 21.0 + i as f32 * 18.0,
                13.0,
                600,
                p.input,
                "start",
                line,
            );
        }
        y += h + 10.0;
    }
    if waiting.len() > MAX_CALLOUTS {
        c.text(
            PAD + 14.0,
            y + 12.0,
            12.0,
            600,
            p.input,
            "start",
            &format!("+{} more need you", waiting.len() - MAX_CALLOUTS),
        );
        y += 24.0;
    }

    // The tree, up to `MAX_AGENTS` of it.
    let (next, drawn) = card_tree(c, graph, root, y, p);
    y = next;
    let hidden = nodes.len() - drawn;
    if hidden > 0 {
        c.text(
            PAD + 22.0,
            y + 8.0,
            12.0,
            400,
            p.faint,
            "start",
            &format!("+{hidden} more agent{}", if hidden == 1 { "" } else { "s" }),
        );
        y += 18.0;
    }

    // The session's own tasks.
    if !root.tasks.is_empty() {
        y += 14.0;
        let done = root.tasks.len() - root.open_tasks;
        c.text(
            PAD,
            y + 11.0,
            11.0,
            700,
            p.muted,
            "start",
            &format!("TASKS · {done}/{} DONE", root.tasks.len()),
        );
        y += 22.0;
        for task in root.tasks.iter().take(MAX_TASKS) {
            let text = match (task.status, &task.active_text) {
                (TaskStatus::InProgress, Some(active)) => active.as_str(),
                _ => task.text.as_str(),
            };
            let (color, weight) = match task.status {
                TaskStatus::Completed => (p.faint, 400),
                TaskStatus::InProgress => (p.text, 600),
                TaskStatus::Pending => (p.muted, 400),
            };
            c.task_icon(PAD + 7.0, y + 9.0, task.status, p);
            let line = fit(text, WIDTH - 2.0 * PAD - 24.0, 13.0, weight > 400);
            c.text(PAD + 22.0, y + 13.5, 13.0, weight, color, "start", &line);
            if task.status == TaskStatus::Completed {
                let w = text_width(&line, 13.0, false);
                c.line(PAD + 22.0, y + 9.0, PAD + 22.0 + w, y + 9.0, p.faint, 1.0);
            }
            y += 22.0;
        }
        if root.tasks.len() > MAX_TASKS {
            c.text(
                PAD + 22.0,
                y + 12.0,
                12.0,
                400,
                p.faint,
                "start",
                &format!("+{} more", root.tasks.len() - MAX_TASKS),
            );
            y += 20.0;
        }
    }
    y
}

/// Draws the cards of `root`'s tree (see `tree_order`) from `y` down, each
/// indented under its parent and joined to it by a connector line. Returns
/// the next free y and how many cards were drawn.
fn card_tree(c: &mut Canvas, graph: &Graph, root: &Node, mut y: f32, p: &Palette) -> (f32, usize) {
    let order = tree_order(graph, root);
    // A deep tree gets a smaller step per level, so every level still shows
    // and no card is indented more than `MAX_INDENT`.
    let deepest = order.iter().map(|(_, depth, _)| *depth).max().unwrap_or(0);
    let step = INDENT.min(MAX_INDENT / deepest.max(1) as f32);
    let mut placed: Vec<(f32, f32)> = Vec::new(); // each card's x and bottom
    for (node, depth, parent) in &order {
        let x = PAD + *depth as f32 * step;
        if let Some((px, bottom)) = parent.map(|i| placed[i]) {
            // Down the gap left of the parent's children, so it passes beside
            // their cards, never through them.
            let spine = px + step / 2.0;
            c.path(&format!("M{spine} {bottom} V{} H{x}", y + 20.0), p.line);
        }
        let height = card(c, graph, node, x, y, p);
        placed.push((x, y + height));
        y += height + 8.0;
    }
    (y, order.len())
}

/// The cards `card_tree` draws, depth first: each node with its depth and
/// its parent's place in the list. At most `MAX_AGENTS` below `root`, and no
/// node twice (the reducer never makes a cycle, but a drawing must end
/// whatever it's given).
fn tree_order<'a>(graph: &'a Graph, root: &'a Node) -> Vec<(&'a Node, usize, Option<usize>)> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    let mut stack = vec![(root, 0, None)];
    while let Some((node, depth, parent)) = stack.pop() {
        if out.len() > MAX_AGENTS {
            break;
        }
        if !seen.insert(node.id.as_str()) {
            continue;
        }
        let at = out.len();
        out.push((node, depth, parent));
        let children = node.children.iter().filter_map(|id| graph.nodes.get(id));
        stack.extend(children.rev().map(|child| (child, depth + 1, Some(at))));
    }
    out
}

/// Draws one card and returns its height.
fn card(c: &mut Canvas, graph: &Graph, n: &Node, x: f32, y: f32, p: &Palette) -> f32 {
    let w = WIDTH - PAD - x;
    let inner = w - 28.0 - 12.0;
    let done = matches!(n.state, State::Completed | State::Canceled);
    // On the name's line, right-aligned: how many tasks are done (with a
    // bar), and whether it runs in the background.
    let progress = (!n.tasks.is_empty()).then(|| {
        let total = n.tasks.len();
        (total - n.open_tasks, total)
    });
    let count = progress.map(|(finished, total)| format!("{finished}/{total}"));
    let progress_w = count.as_deref().map_or(0.0, |count| {
        text_width(count, 12.0, false) + 6.0 + 44.0 + 8.0
    });
    let background = n.background == Some(true);
    let background_w = if background {
        text_width("background", 11.0, false) + 8.0
    } else {
        0.0
    };

    // Lay out the text first so we know the height.
    let name = if n.kind == NodeKind::Session {
        crate::render::card_name(n)
    } else {
        name(n)
    };
    let name = fit(
        &name,
        inner - progress_w - background_w - 8.0 - pill_width(n.state, n.stale),
        14.0,
        true,
    );
    let mut body: Vec<(String, f32, u16, &str)> = Vec::new(); // text, size, weight, colour
    // Why it was started: for agents, and sessions another started.
    if n.kind == NodeKind::Agent || n.parent.is_some() {
        if let Some(purpose) = &n.purpose {
            for line in wrap(purpose, inner, 13.0, false, 2) {
                body.push((line, 13.0, 400, p.text));
            }
        }
    }
    if n.state == State::InputRequired {
        let msg = format!(
            "Needs you: {}",
            n.attention.as_deref().unwrap_or("waiting for input")
        );
        for line in wrap(&msg, inner, 13.0, true, 2) {
            body.push((line, 13.0, 600, p.input));
        }
    } else if let Some(headline) = &n.headline {
        for line in wrap(headline, inner, 13.0, false, 2) {
            body.push((line, 13.0, 400, p.muted));
        }
    }
    if let Some(b) = &n.blocked {
        for line in wrap(&blocked_text(graph, b), inner, 12.5, false, 2) {
            body.push((line, 12.5, 500, p.working));
        }
    }
    let height = 40.0 + body.len() as f32 * 18.0;

    let (stroke, width) = match n.state {
        State::InputRequired => (p.input, 1.5),
        _ => (p.line, 1.0),
    };
    c.rect(
        x,
        y,
        w,
        height,
        10.0,
        if done { p.panel_done } else { p.panel },
        Some((stroke, width)),
    );
    c.circle(x + 16.0, y + 20.0, 4.5, state_color(n.state, p));
    c.text(
        x + 28.0,
        y + 25.0,
        14.0,
        650,
        if done { p.muted } else { p.text },
        "start",
        &name,
    );
    c.pill(
        x + 28.0 + text_width(&name, 14.0, true) + 8.0,
        y + 11.0,
        n.state,
        n.stale,
        p,
    );
    if background {
        // Marked so it's clear the parent isn't waiting for it.
        c.text(
            x + w - 12.0 - progress_w,
            y + 25.0,
            11.0,
            500,
            p.faint,
            "end",
            "background",
        );
    }
    if let (Some((finished, total)), Some(count)) = (progress, &count) {
        let bx = x + w - 12.0 - 44.0;
        c.text(bx - 6.0, y + 25.0, 12.0, 500, p.muted, "end", count);
        c.rect(bx, y + 18.0, 44.0, 5.0, 2.5, p.line, None);
        if finished > 0 {
            c.rect(
                bx,
                y + 18.0,
                44.0 * finished as f32 / total as f32,
                5.0,
                2.5,
                p.completed,
                None,
            );
        }
    }
    for (i, (text, size, weight, color)) in body.iter().enumerate() {
        c.text(
            x + 28.0,
            y + 44.0 + i as f32 * 18.0,
            *size,
            *weight,
            color,
            "start",
            text,
        );
    }
    height
}

fn blocked_text(graph: &Graph, b: &Blocked) -> String {
    let mut parts = Vec::new();
    if b.cycle {
        parts.push("Deadlock".to_string());
    }
    if !b.on.is_empty() {
        let mut names: Vec<String> =
            b.on.iter()
                .take(2)
                .map(|id| match graph.nodes.get(id) {
                    // A stale target is usually why the wait is long.
                    Some(n) if n.stale => format!("{} (looks stuck)", name(n)),
                    Some(n) => name(n),
                    None => short(local(id)).into(),
                })
                .collect();
        if b.on.len() > 2 {
            names.push(format!("{} more", b.on.len() - 2));
        }
        parts.push(format!("Waiting on {}", names.join(", ")));
    }
    if b.starting > 0 {
        parts.push(format!("{} starting", plural(b.starting, "agent")));
    }
    if b.open_tasks > 0 {
        parts.push(format!("{} ahead", plural(b.open_tasks, "open task")));
    }
    parts.join(" · ")
}

fn state_color(state: State, p: &Palette) -> &str {
    match state {
        State::Working => p.working,
        State::InputRequired => p.input,
        State::Idle => p.idle,
        State::Completed => p.completed,
        State::Failed => p.failed,
        State::Canceled => p.faint,
    }
}

/// How wide `Canvas::pill` draws, so text beside it can leave room.
fn pill_width(state: State, stale: bool) -> f32 {
    let w = text_width(state_label(state), 11.0, true) + 16.0;
    if stale {
        w + 8.0 + text_width("stale?", 11.5, true)
    } else {
        w
    }
}

fn state_label(state: State) -> &'static str {
    match state {
        State::Working => "Working",
        State::InputRequired => "Needs you",
        State::Idle => "Idle",
        State::Completed => "Completed",
        State::Failed => "Failed",
        State::Canceled => "Canceled",
    }
}

fn subtree<'a>(graph: &'a Graph, root: &'a Node) -> Vec<&'a Node> {
    let mut out = vec![root];
    let mut seen = BTreeSet::from([root.id.as_str()]);
    let mut i = 0;
    while i < out.len() {
        let kids: Vec<&Node> = out[i]
            .children
            .iter()
            .filter_map(|id| graph.nodes.get(id))
            .filter(|n| seen.insert(n.id.as_str()))
            .collect();
        out.extend(kids);
        i += 1;
    }
    out
}

fn name(n: &Node) -> String {
    format!(
        "{} {}",
        n.agent_type.as_deref().unwrap_or("Agent"),
        short(local(&n.id))
    )
}

fn local(id: &str) -> &str {
    id.rsplit(['/', ':']).next().unwrap_or(id)
}

fn short(id: &str) -> &str {
    id.char_indices().nth(8).map_or(id, |(i, _)| &id[..i])
}

fn plural(n: usize, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}

// ---------- text measurement ----------
//
// We can't measure glyphs before rendering, so widths are estimated from
// per-character classes. The estimates run slightly wide, so text wraps a
// little early rather than overflowing its card.

fn text_width(s: &str, size: f32, bold: bool) -> f32 {
    let em: f32 = s
        .chars()
        .map(|ch| match ch {
            'i' | 'l' | 'j' | '|' | '.' | ',' | ':' | ';' | '\'' | '!' | 'I' => 0.28,
            'f' | 't' | 'r' | ' ' | '(' | ')' | '[' | ']' | '/' | '-' => 0.36,
            'm' | 'w' | 'M' | 'W' | '@' => 0.86,
            c if c.is_ascii_uppercase() => 0.66,
            c if c.is_ascii_digit() => 0.56,
            c if c.is_ascii() => 0.54,
            // Wide scripts (CJK and the like) take a full em.
            c if c as u32 >= 0x2E80 => 1.0,
            _ => 0.6,
        })
        .sum();
    em * size * if bold { 1.07 } else { 1.0 }
}

/// Word-wraps `s` to `max_width`, keeping at most `max_lines` and ending the
/// last with `…` if anything was cut.
fn wrap(s: &str, max_width: f32, size: f32, bold: bool, max_lines: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut words = s.split_whitespace().peekable();
    while let Some(word) = words.next() {
        let candidate = if current.is_empty() {
            word.to_string()
        } else {
            format!("{current} {word}")
        };
        if text_width(&candidate, size, bold) <= max_width {
            current = candidate;
            continue;
        }
        if !current.is_empty() {
            lines.push(std::mem::take(&mut current));
        }
        if lines.len() == max_lines {
            let last = lines.pop().unwrap_or_default();
            lines.push(fit(&format!("{last} {word}…"), max_width, size, bold));
            return lines;
        }
        // A single word longer than the line gets cut.
        current = if text_width(word, size, bold) > max_width {
            fit(word, max_width, size, bold)
        } else {
            word.to_string()
        };
        if words.peek().is_none() {
            break;
        }
    }
    if !current.is_empty() {
        if lines.len() == max_lines {
            let last = lines.pop().unwrap_or_default();
            lines.push(fit(&format!("{last} {current}"), max_width, size, bold));
        } else {
            lines.push(current);
        }
    }
    lines
}

/// Cuts `s` to fit `max_width`, ending with `…` if anything was removed.
fn fit(s: &str, max_width: f32, size: f32, bold: bool) -> String {
    if text_width(s, size, bold) <= max_width {
        return s.to_string();
    }
    let mut out = String::new();
    for ch in s.chars() {
        let next = format!("{out}{ch}…");
        if text_width(&next, size, bold) > max_width {
            break;
        }
        out.push(ch);
    }
    format!("{}…", out.trim_end())
}

// ---------- SVG output ----------

#[derive(Default)]
struct Canvas {
    out: String,
}

impl Canvas {
    #[allow(clippy::too_many_arguments)]
    fn rect(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        r: f32,
        fill: &str,
        stroke: Option<(&str, f32)>,
    ) {
        let stroke = stroke
            .map(|(c, sw)| format!(" stroke=\"{c}\" stroke-width=\"{sw}\""))
            .unwrap_or_default();
        let _ = write!(
            self.out,
            "<rect x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\" rx=\"{r}\" fill=\"{fill}\"{stroke}/>"
        );
    }

    fn circle(&mut self, cx: f32, cy: f32, r: f32, fill: &str) {
        let _ = write!(
            self.out,
            "<circle cx=\"{cx}\" cy=\"{cy}\" r=\"{r}\" fill=\"{fill}\"/>"
        );
    }

    fn line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, color: &str, width: f32) {
        let _ = write!(
            self.out,
            "<line x1=\"{x1}\" y1=\"{y1}\" x2=\"{x2}\" y2=\"{y2}\" stroke=\"{color}\" stroke-width=\"{width}\"/>"
        );
    }

    fn path(&mut self, d: &str, color: &str) {
        let _ = write!(
            self.out,
            "<path d=\"{d}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"1.5\"/>"
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn text(
        &mut self,
        x: f32,
        y: f32,
        size: f32,
        weight: u16,
        fill: &str,
        anchor: &str,
        content: &str,
    ) {
        let _ = write!(
            self.out,
            "<text x=\"{x}\" y=\"{y}\" font-size=\"{size}\" font-weight=\"{weight}\" fill=\"{fill}\" \
             text-anchor=\"{anchor}\">{}</text>",
            escape(content)
        );
    }

    /// Draws the state pill, plus a "stale?" flag when `stale`: `pill_width`
    /// in all.
    fn pill(&mut self, x: f32, y: f32, state: State, stale: bool, p: &Palette) {
        let label = state_label(state);
        let color = state_color(state, p);
        let w = pill_width(state, false);
        if stale {
            self.text(
                x + w + 8.0,
                y + 13.5,
                11.5,
                650,
                p.failed,
                "start",
                "stale?",
            );
        }
        let _ = write!(
            self.out,
            "<rect x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"19\" rx=\"9.5\" fill=\"{color}\" fill-opacity=\"0.14\"/>"
        );
        self.text(x + w / 2.0, y + 13.5, 11.0, 650, color, "middle", label);
    }

    fn task_icon(&mut self, cx: f32, cy: f32, status: TaskStatus, p: &Palette) {
        match status {
            TaskStatus::Completed => {
                self.circle(cx, cy, 7.0, p.completed);
                let _ = write!(
                    self.out,
                    "<path d=\"M{} {} l2.2 2.2 l4.2 -4.4\" fill=\"none\" stroke=\"{}\" stroke-width=\"1.8\" \
                     stroke-linecap=\"round\" stroke-linejoin=\"round\"/>",
                    cx - 3.3,
                    cy,
                    p.panel
                );
            }
            TaskStatus::InProgress => {
                let _ = write!(
                    self.out,
                    "<circle cx=\"{cx}\" cy=\"{cy}\" r=\"6.2\" fill=\"none\" stroke=\"{0}\" stroke-width=\"1.6\"/>\
                     <circle cx=\"{cx}\" cy=\"{cy}\" r=\"3\" fill=\"{0}\"/>",
                    p.working
                );
            }
            TaskStatus::Pending => {
                let _ = write!(
                    self.out,
                    "<circle cx=\"{cx}\" cy=\"{cy}\" r=\"6.2\" fill=\"none\" stroke=\"{}\" stroke-width=\"1.6\"/>",
                    p.faint
                );
            }
        }
    }
}

fn escape(s: &str) -> String {
    let s = crate::render::clean(s);
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // Characters XML doesn't allow: control characters (bar tab),
            // and the noncharacters U+FFFE and U+FFFF.
            c if (c as u32) < 0x20 && c != '\t' => out.push(' '),
            '\u{FFFE}' | '\u{FFFF}' => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_model_text() {
        assert_eq!(
            escape("<script>&\"'\u{1}"),
            "&lt;script&gt;&amp;&quot;&apos; "
        );
    }

    /// R33: XML forbids U+FFFE and U+FFFF; one in any text used to make
    /// the whole picture fail to render.
    #[test]
    fn escapes_characters_xml_forbids() {
        let mut g = Graph {
            nodes: Default::default(),
            roots: vec![],
            late: Default::default(),
        };
        let mut root = node("p:s", NodeKind::Session, None);
        root.title = Some("title \u{FFFE}\u{FFFF}".into());
        root.headline = Some("\u{FFFF}".into());
        g.nodes.insert(root.id.clone(), root);
        let svg = svg(&g, &["p:s".into()], &test_options());
        png(&svg).expect("renders");
        assert_eq!(escape("a\u{FFFE}b\u{FFFF}c\u{0}\u{1F}\u{7F}"), "a b c   ");
    }

    fn test_options() -> Options {
        Options {
            theme: Theme::Light,
            as_of: SystemTime::UNIX_EPOCH,
        }
    }

    fn node(id: &str, kind: NodeKind, parent: Option<&str>) -> Node {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "kind": kind,
            "provider": "p",
            "parent": parent,
            "children": [],
            "state": "working",
            "tasks": [],
            "spawns": [],
            "waits": [],
            "messages": [],
            "last_event_at": "2026-01-01T00:00:00Z",
            "open_tasks": 0,
            "stale": false,
        }))
        .unwrap()
    }

    /// A graph of `nodes`, each the child of its `parent`.
    fn graph_of(nodes: Vec<Node>) -> Graph {
        let mut g = Graph {
            nodes: Default::default(),
            roots: vec![],
            late: Default::default(),
        };
        for n in nodes {
            match &n.parent {
                Some(parent) => g.nodes.get_mut(parent).unwrap().children.push(n.id.clone()),
                None => g.roots.push(n.id.clone()),
            }
            g.nodes.insert(n.id.clone(), n);
        }
        g
    }

    /// Checks the drawing of `g` (as far as estimated text widths can tell):
    /// no two pieces of text overlap, all of it is inside the margins, every
    /// shape has a positive size, no connector runs through a card, and it
    /// renders.
    fn assert_laid_out(g: &Graph) -> String {
        let svg = svg(g, &g.roots, &test_options());
        let attr = |tag: &str, name: &str| -> Option<f32> {
            let tag = format!(" {tag}");
            let at = tag.find(&format!(" {name}=\""))? + name.len() + 3;
            tag[at..].split('"').next()?.parse().ok()
        };
        let mut cards: Vec<(f32, f32, f32, f32)> = Vec::new(); // left, right, top, bottom
        for part in svg.split("<rect ").skip(1) {
            let tag = part.split_once('>').unwrap().0;
            for dim in ["width", "height"] {
                if let Some(v) = attr(tag, dim) {
                    assert!(v > 0.0, "a rect has {dim} {v}: {tag}");
                }
            }
            if attr(tag, "rx") == Some(10.0) {
                let (x, y) = (attr(tag, "x").unwrap(), attr(tag, "y").unwrap());
                let (w, h) = (attr(tag, "width").unwrap(), attr(tag, "height").unwrap());
                cards.push((x, x + w, y, y + h));
            }
        }
        // Connectors are "M{x} {y} V{y}", then maybe " H{x}".
        for part in svg.split("<path d=\"M").skip(1) {
            let d = part.split('"').next().unwrap();
            let Some((start, rest)) = d.split_once(" V") else {
                continue;
            };
            let num = |s: &str| -> f32 { s.parse().unwrap() };
            let (x0, y0) = start.split_once(' ').unwrap();
            let (x0, y0) = (num(x0), num(y0));
            let (y1, x1) = match rest.split_once(" H") {
                Some((y1, x1)) => (num(y1), num(x1)),
                None => (num(rest), x0),
            };
            // Each piece as (left, right, top, bottom); touching an edge is
            // fine, entering the card isn't.
            for seg in [
                (x0, x0, y0.min(y1), y0.max(y1)),
                (x0.min(x1), x0.max(x1), y1, y1),
            ] {
                for card in &cards {
                    let crosses =
                        seg.0 < card.1 && seg.1 > card.0 && seg.2 < card.3 && seg.3 > card.2;
                    assert!(!crosses, "connector M{d} runs through the card at {card:?}");
                }
            }
        }
        let mut boxes: Vec<(f32, f32, f32, f32, String)> = Vec::new();
        for part in svg.split("<text ").skip(1) {
            let (tag, rest) = part.split_once('>').unwrap();
            let content = rest.split("</text>").next().unwrap();
            let content = content
                .replace("&lt;", "<")
                .replace("&gt;", ">")
                .replace("&quot;", "\"")
                .replace("&apos;", "'")
                .replace("&amp;", "&");
            let (x, y, size) = (
                attr(tag, "x").unwrap(),
                attr(tag, "y").unwrap(),
                attr(tag, "font-size").unwrap(),
            );
            let w = text_width(&content, size, attr(tag, "font-weight").unwrap() >= 600.0);
            let left = if tag.contains("text-anchor=\"end\"") {
                x - w
            } else if tag.contains("text-anchor=\"middle\"") {
                x - w / 2.0
            } else {
                x
            };
            boxes.push((left, left + w, y - size, y, content));
        }
        for (i, a) in boxes.iter().enumerate() {
            assert!(
                a.0 >= PAD - 0.01 && a.1 <= WIDTH - PAD + 0.01,
                "{:?} is outside the margins ({}..{})",
                a.4,
                a.0,
                a.1
            );
            for b in &boxes[i + 1..] {
                let apart = a.1 <= b.0 || b.1 <= a.0 || a.3 <= b.2 || b.3 <= a.2;
                assert!(apart, "{:?} overlaps {:?}", a.4, b.4);
            }
        }
        png(&svg).expect("renders");
        svg
    }

    fn agent(id: &str, parent: &str) -> Node {
        let mut n = node(id, NodeKind::Agent, Some(parent));
        n.agent_type = Some("general-purpose".into());
        n
    }

    /// R40: the "stale?" flag used to be drawn after the state pill without
    /// room kept for it, over a card's "background" and progress, and past
    /// the header's right margin.
    #[test]
    fn the_stale_flag_has_room() {
        let mut root = node("p:s", NodeKind::Session, None);
        root.title = Some("W".repeat(80));
        root.state = State::InputRequired;
        root.stale = true;
        let mut child = agent("p:s/a", "p:s");
        child.agent_type = Some("M".repeat(80));
        child.state = State::InputRequired;
        child.stale = true;
        child.background = Some(true);
        child.tasks = (0..10)
            .map(|i| crate::reducer::Task {
                id: i.to_string(),
                text: "t".into(),
                active_text: None,
                status: TaskStatus::Pending,
            })
            .collect();
        child.open_tasks = 10;
        assert_laid_out(&graph_of(vec![root, child]));
    }

    /// R40: each level of the tree is indented, so a deep enough one used to
    /// give cards negative widths.
    #[test]
    fn a_deep_tree_stays_inside_the_picture() {
        let mut nodes = vec![node("p:s", NodeKind::Session, None)];
        for i in 0..30 {
            let parent = nodes.last().unwrap().id.clone();
            let mut n = agent(&format!("p:s/{i:08}"), &parent);
            n.stale = true;
            nodes.push(n);
        }
        let svg = assert_laid_out(&graph_of(nodes));
        assert!(svg.contains(">general-purpose 00000029<"), "all are drawn");
    }

    /// R40: a session with thousands of agents drew them all, making a
    /// picture too tall to render or share. It draws the first few and
    /// counts the rest.
    #[test]
    fn a_huge_session_draws_some_agents_and_counts_the_rest() {
        let mut nodes = vec![node("p:s", NodeKind::Session, None)];
        for i in 0..5000 {
            let mut n = agent(&format!("p:s/{i:08}"), "p:s");
            n.state = State::InputRequired;
            nodes.push(n);
        }
        let g = graph_of(nodes);
        let svg = svg(&g, &g.roots, &test_options());
        assert!(svg.len() < 200_000, "{} bytes", svg.len());
        let cards = svg.matches(">general-purpose 0").count() - MAX_CALLOUTS;
        assert_eq!(cards, MAX_AGENTS);
        assert!(svg.contains(">+4970 more agents<"), "the rest are counted");
        assert_eq!(svg.matches(" needs you: ").count(), MAX_CALLOUTS);
        assert!(
            svg.contains(">+4997 more need you<"),
            "so are their callouts"
        );
        assert_laid_out(&g);
    }

    /// R40: the count of agents not drawn appears only when there are some.
    #[test]
    fn agents_past_the_cap_are_counted_exactly() {
        let session = |agents: usize| {
            let mut nodes = vec![node("p:s", NodeKind::Session, None)];
            for i in 0..agents {
                nodes.push(agent(&format!("p:s/{i:08}"), "p:s"));
            }
            assert_laid_out(&graph_of(nodes))
        };
        let svg = session(MAX_AGENTS);
        assert!(!svg.contains(" more agent"), "all {MAX_AGENTS} are drawn");
        assert!(svg.contains(&format!(">general-purpose {:08}<", MAX_AGENTS - 1)));
        let svg = session(MAX_AGENTS + 1);
        assert!(svg.contains(">+1 more agent<"));
        assert!(!svg.contains(&format!(">general-purpose {:08}<", MAX_AGENTS)));
    }

    /// R40: past the depth where cards used to stop being indented, a chain
    /// and a node with several children must still look different, and no
    /// connector may run through a card.
    #[test]
    fn a_deep_tree_keeps_its_shape() {
        let mut nodes = vec![node("p:s", NodeKind::Session, None)];
        for i in 0..14 {
            let parent = nodes.last().unwrap().id.clone();
            nodes.push(agent(&format!("p:s/a{i:07}"), &parent));
        }
        let deep = nodes.last().unwrap().id.clone();
        nodes.push(agent("p:s/b1", &deep));
        nodes.push(agent("p:s/c1", "p:s/b1"));
        nodes.push(agent("p:s/b2", &deep));
        let svg = assert_laid_out(&graph_of(nodes));
        // Cards in drawing order: the session, a0..a13, b1, c1, b2.
        let xs: Vec<f32> = svg
            .split("<rect ")
            .filter(|r| r.contains(" rx=\"10\""))
            .map(|r| r.split('"').nth(1).unwrap().parse().unwrap())
            .collect();
        assert_eq!(xs.len(), 18);
        assert!(xs[..17].windows(2).all(|w| w[0] < w[1]), "{xs:?}");
        assert_eq!(xs[15], xs[17], "b1 and b2 are siblings: {xs:?}");
    }

    #[test]
    fn wraps_and_ellipsizes() {
        let lines = wrap(
            "one two three four five six seven eight nine ten",
            60.0,
            13.0,
            false,
            2,
        );
        assert_eq!(lines.len(), 2);
        assert!(lines[1].ends_with('…'));
        assert!(
            lines
                .iter()
                .all(|l| text_width(l, 13.0, false) <= 60.0 + 0.01)
        );
        assert_eq!(
            wrap("short", 200.0, 13.0, false, 2),
            vec!["short".to_string()]
        );
    }

    #[test]
    fn fit_cuts_long_words() {
        let s = fit(&"x".repeat(200), 80.0, 13.0, false);
        assert!(s.ends_with('…'));
        assert!(text_width(&s, 13.0, false) <= 80.0);
    }
}
