//! `agent-graph tail`: the graph, redrawn in the terminal as events arrive.
//!
//! In a terminal it takes over the screen like `top` (q quits). When stdout
//! isn't a terminal it prints a new frame each time something changes, so it
//! can be piped or logged.

use std::io::{self, IsTerminal, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::style::{Attribute, Color, ContentStyle, PrintStyledContent, StyledContent};
use crossterm::terminal::{self, ClearType};
use crossterm::{cursor, execute, queue};

use crate::event::State;
use crate::reducer::{self, Graph};
use crate::render::{self, Line, Span, Tone};
use crate::view::tail::Tail;

pub struct Options {
    /// Follow one session (an id or a unique prefix of one).
    pub session: Option<String>,
    /// Start with older sessions shown.
    pub all: bool,
    pub ascii: bool,
    pub stale_after: Duration,
}

/// How often to check for new events and key presses.
const TICK: Duration = Duration::from_millis(250);
/// Redraw at least this often, so staleness keeps up with the clock.
const REFRESH: Duration = Duration::from_secs(5);
const RECENT: Duration = Duration::from_secs(24 * 60 * 60);

pub fn run(events_dir: &Path, opts: Options) -> Result<(), String> {
    let mut tail = Tail::new(events_dir);
    tail.poll()
        .map_err(|e| format!("reading {}: {e}", events_dir.display()))?;
    if !io::stdout().is_terminal() {
        return frames(&mut tail, &opts);
    }

    let _screen = Screen::enter()?;
    let mut all = opts.all;
    let mut dirty = true;
    let mut drawn = Instant::now();
    loop {
        if event::poll(TICK).map_err(term_err)? {
            match event::read().map_err(term_err)? {
                Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return Ok(());
                    }
                    KeyCode::Char('a') => {
                        all = !all;
                        dirty = true;
                    }
                    _ => {}
                },
                Event::Resize(..) => dirty = true,
                _ => {}
            }
        }
        // A read error (say, the directory is briefly missing) is retried.
        if tail.poll().unwrap_or(false) {
            dirty = true;
        }
        if dirty || drawn.elapsed() >= REFRESH {
            let (width, height) = terminal::size().map_err(term_err)?;
            let frame = frame(&tail, &opts, all, true)?;
            draw(&frame, width as usize, height as usize, all).map_err(term_err)?;
            dirty = false;
            drawn = Instant::now();
        }
    }
}

/// Not a terminal: print the whole picture whenever it changes.
fn frames(tail: &mut Tail, opts: &Options) -> Result<(), String> {
    let mut out = io::stdout();
    let mut first = true;
    loop {
        if first || tail.poll().unwrap_or(false) {
            let mut text = if first {
                String::new()
            } else {
                "\n".to_string()
            };
            first = false;
            for line in frame(tail, opts, opts.all, false)? {
                text.push_str(&plain(&line));
                text.push('\n');
            }
            // Stop quietly when the reader goes away (e.g. `| head`).
            if out
                .write_all(text.as_bytes())
                .and_then(|_| out.flush())
                .is_err()
            {
                return Ok(());
            }
        }
        std::thread::sleep(TICK);
    }
}

/// The header, then each session's tree with a blank line between them.
fn frame(tail: &Tail, opts: &Options, all: bool, live: bool) -> Result<Vec<Line>, String> {
    let now = crate::clock::now();
    let graph = reducer::reduce(
        tail.events.iter().map(|t| t.event.clone()).collect(),
        &reducer::Options {
            now,
            stale_after: opts.stale_after,
        },
    );
    let roots = match &opts.session {
        Some(want) => {
            let cwd = std::env::current_dir().ok();
            crate::cli::pick_roots(&graph, Some(want), false, cwd.as_deref())?
        }
        None => graph
            .roots
            .iter()
            .filter(|id| {
                all || humantime::parse_rfc3339_weak(&graph.nodes[*id].last_event_at)
                    .is_ok_and(|t| now.duration_since(t).unwrap_or_default() <= RECENT)
            })
            .cloned()
            .collect(),
    };

    let mut lines = vec![
        header(&graph, &roots, tail.events.len(), now, live),
        Vec::new(),
    ];
    if roots.is_empty() {
        let hint = if graph.roots.is_empty() {
            "No sessions yet. Run `agent-graph install claude-code`, then start a new Claude Code session."
        } else {
            "Nothing in the last 24 hours. Press a to show older sessions."
        };
        lines.push(vec![Span {
            text: hint.into(),
            tone: Tone::Dim,
        }]);
    }
    let branches = if opts.ascii {
        render::ASCII
    } else {
        render::UNICODE
    };
    for (i, root) in roots.iter().enumerate() {
        if i > 0 {
            lines.push(Vec::new());
        }
        lines.extend(render::lines(&graph, std::slice::from_ref(root), branches));
    }
    Ok(lines)
}

fn header(
    graph: &Graph,
    roots: &[String],
    events: usize,
    now: std::time::SystemTime,
    live: bool,
) -> Line {
    let nodes: Vec<_> = roots.iter().flat_map(|r| subtree(graph, r)).collect();
    let needs_you = nodes
        .iter()
        .filter(|n| n.state == State::InputRequired)
        .count();
    let stale = nodes.iter().filter(|n| n.stale).count();
    let deadlocked = nodes
        .iter()
        .filter(|n| n.blocked.as_ref().is_some_and(|b| b.cycle))
        .count();
    let working = nodes.iter().filter(|n| n.state == State::Working).count();

    let s = |text: String, tone| Span { text, tone };
    let clock = humantime::format_rfc3339_seconds(now).to_string();
    let clock = clock.get(11..19).unwrap_or_default();
    let mut line = vec![s("Agent Graph".into(), Tone::Strong)];
    if live {
        line.push(s("  ● live".into(), Tone::Good));
    }
    line.push(s(format!("  {clock} UTC"), Tone::Dim));
    line.push(s(
        format!(
            "  {} session{} · {working} working · {events} events",
            roots.len(),
            plural(roots.len())
        ),
        Tone::Dim,
    ));
    if needs_you > 0 {
        line.push(s(
            format!(
                "  {needs_you} need{} you",
                if needs_you == 1 { "s" } else { "" }
            ),
            Tone::Attention,
        ));
    }
    if stale > 0 {
        line.push(s(format!("  {stale} stale?"), Tone::Bad));
    }
    if deadlocked > 0 {
        line.push(s(format!("  {deadlocked} deadlocked"), Tone::Bad));
    }
    line
}

fn subtree<'a>(graph: &'a Graph, root: &str) -> Vec<&'a crate::reducer::Node> {
    let mut out: Vec<&crate::reducer::Node> = graph.nodes.get(root).into_iter().collect();
    let mut i = 0;
    while i < out.len() {
        let kids = out[i].children.iter().filter_map(|c| graph.nodes.get(c));
        out.extend(kids);
        i += 1;
    }
    out
}

/// Draws `frame` over the previous one without clearing first, so there's
/// no flicker. Lines are cut to the width; extra lines are counted.
fn draw(frame: &[Line], width: usize, height: usize, all: bool) -> io::Result<()> {
    let mut out = io::stdout();
    queue!(out, cursor::MoveTo(0, 0))?;
    let body = height.saturating_sub(1);
    let overflow = frame.len() > body;
    let shown = if overflow {
        body.saturating_sub(1)
    } else {
        frame.len()
    };
    for line in &frame[..shown] {
        print_line(&mut out, line, width)?;
        queue!(
            out,
            terminal::Clear(ClearType::UntilNewLine),
            cursor::MoveToNextLine(1)
        )?;
    }
    if overflow {
        let more = vec![Span {
            text: format!("… {} more lines", frame.len() - shown),
            tone: Tone::Dim,
        }];
        print_line(&mut out, &more, width)?;
        queue!(out, cursor::MoveToNextLine(1))?;
    }
    queue!(out, terminal::Clear(ClearType::FromCursorDown))?;
    let help = format!(
        "q quit · a {} older sessions",
        if all { "hide" } else { "show" }
    );
    queue!(out, cursor::MoveTo(0, height.saturating_sub(1) as u16))?;
    print_line(
        &mut out,
        &[Span {
            text: help,
            tone: Tone::Dim,
        }],
        width,
    )?;
    out.flush()
}

fn print_line(out: &mut impl Write, line: &[Span], width: usize) -> io::Result<()> {
    let mut left = width;
    for span in line {
        if left == 0 {
            break;
        }
        let text: String = span.text.chars().take(left).collect();
        left -= text.chars().count();
        queue!(
            out,
            PrintStyledContent(StyledContent::new(style(span.tone), text))
        )?;
    }
    Ok(())
}

fn style(tone: Tone) -> ContentStyle {
    let mut s = ContentStyle::new();
    match tone {
        Tone::Plain => {}
        Tone::Dim => s.foreground_color = Some(Color::DarkGrey),
        Tone::Strong => s.attributes.set(Attribute::Bold),
        Tone::Working => s.foreground_color = Some(Color::Cyan),
        Tone::Attention => {
            s.foreground_color = Some(Color::Yellow);
            s.attributes.set(Attribute::Bold);
        }
        Tone::Good => s.foreground_color = Some(Color::Green),
        Tone::Bad => {
            s.foreground_color = Some(Color::Red);
            s.attributes.set(Attribute::Bold);
        }
    }
    s
}

fn plain(line: &[Span]) -> String {
    line.iter().map(|s| s.text.as_str()).collect()
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

fn term_err(e: io::Error) -> String {
    format!("terminal: {e}")
}

/// Owns the terminal while `tail` runs, and restores it however we exit,
/// including on a panic.
struct Screen;

impl Screen {
    fn enter() -> Result<Screen, String> {
        terminal::enable_raw_mode().map_err(term_err)?;
        let screen = Screen;
        execute!(io::stdout(), terminal::EnterAlternateScreen, cursor::Hide).map_err(term_err)?;
        Ok(screen)
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), cursor::Show, terminal::LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
    }
}
