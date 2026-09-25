//! Renders the graph as an image sized for a phone screen: SVG first, then
//! PNG through `resvg` (pure Rust, so it works the same on every platform).
//!
//! Event text comes from models, so everything written into the SVG is escaped.

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
            .cwd
            .as_deref()
            .and_then(|cwd| cwd.rsplit(['/', '\\']).find(|s| !s.is_empty()))
            .unwrap_or("Session")
            .to_string(),
        NodeKind::Agent => name(root),
    };
    let title = fit(&title, WIDTH - 2.0 * PAD - 110.0, 20.0, true);
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
    for node in nodes.iter().filter(|n| n.state == State::InputRequired) {
        let who = if node.kind == NodeKind::Session {
            "Session".to_string()
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

    // The tree.
    y = card_tree(c, graph, root, PAD, y, p);

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

/// Draws `node`'s card at (x, y) and its children below, indented and joined
/// by connector lines. Returns the next free y.
fn card_tree(c: &mut Canvas, graph: &Graph, node: &Node, x: f32, y: f32, p: &Palette) -> f32 {
    let height = card(c, graph, node, x, y, p);
    let mut next = y + height + 8.0;
    let spine = x + 11.0;
    for child in node.children.iter().filter_map(|id| graph.nodes.get(id)) {
        let child_y = next;
        c.path(
            &format!(
                "M{spine} {} V{} H{}",
                y + height,
                child_y + 20.0,
                x + INDENT
            ),
            p.line,
        );
        next = card_tree(c, graph, child, x + INDENT, child_y, p);
    }
    next
}

/// Draws one card and returns its height.
fn card(c: &mut Canvas, graph: &Graph, n: &Node, x: f32, y: f32, p: &Palette) -> f32 {
    let w = WIDTH - PAD - x;
    let inner = w - 28.0 - 12.0;
    let done = matches!(n.state, State::Completed | State::Canceled);
    let progress_w = if n.tasks.is_empty() { 0.0 } else { 64.0 };

    // Lay out the text first so we know the height.
    let name = if n.kind == NodeKind::Session {
        "Session".to_string()
    } else {
        name(n)
    };
    let name = fit(&name, inner - progress_w - 90.0, 14.0, true);
    let mut body: Vec<(String, f32, u16, &str)> = Vec::new(); // text, size, weight, colour
    if n.kind == NodeKind::Agent {
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
    if n.background == Some(true) {
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
    if !n.tasks.is_empty() {
        let total = n.tasks.len();
        let finished = total - n.open_tasks;
        let bx = x + w - 12.0 - 44.0;
        c.text(
            bx - 6.0,
            y + 25.0,
            12.0,
            500,
            p.muted,
            "end",
            &format!("{finished}/{total}"),
        );
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
    let mut i = 0;
    while i < out.len() {
        let kids = out[i].children.iter().filter_map(|id| graph.nodes.get(id));
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

    /// Draws the state pill (plus a "stale?" flag when `stale`) and returns
    /// how wide it all is.
    fn pill(&mut self, x: f32, y: f32, state: State, stale: bool, p: &Palette) -> f32 {
        let label = state_label(state);
        let color = state_color(state, p);
        let w = text_width(label, 11.0, true) + 16.0;
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
        if stale {
            w + 8.0 + text_width("stale?", 11.5, true)
        } else {
            w
        }
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
            // Control characters aren't allowed in XML.
            c if (c as u32) < 0x20 && c != '\t' => out.push(' '),
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
