//! Every subcommand except `emit`, which `main` handles before clap runs.

use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, SystemTime};

use clap::{Parser, Subcommand, ValueEnum};
use serde_json::Value;

use crate::install::{self, Scope};
use crate::reducer::{self, Graph};
use crate::slash::{self, Client};
use crate::{paths, render, store};

const ABOUT: &str = "Record, watch and share a live graph of your AI coding agents.";

const LONG_ABOUT: &str = "\
Record, watch and share a live graph of your AI coding agents.

Agent Graph records what your coding agents do, from their own hooks, and
shows how the sessions relate:

  - which session started which agents, and who is waiting on whom
  - what needs you: permission prompts, questions, plans to approve
  - what looks stuck or deadlocked, and how much work is left
  - what the sessions sent each other

Everything is recorded on this machine. Nothing leaves it unless you share it,
with `watch-remote` or at https://agentgraph.chofter.com.";

const AFTER_HELP: &str = "\
Get started: `agent-graph install claude-code`, then start a new Claude Code session.
Run `agent-graph --help` for examples, where data lives, and settings.";

const AFTER_LONG_HELP: &str = "\
Getting started:
  1. agent-graph install claude-code   Add the recording hooks and the /agent-graph command
  2. Start a new Claude Code session    (sessions already running aren't recorded)
  3. agent-graph tail                  Watch it live in this terminal,
     agent-graph view --open           or in your browser

Examples:
  agent-graph tree --all                         Every session, once, as text
  agent-graph snapshot                           A PNG of this session, sized for a phone
  agent-graph watch-remote --password=hunter2    Share it live; prints a link
  agent-graph install codex                      Add the command to Codex as well

Where things live:
  ~/.agent-graph/events/   One JSON Lines file per session
  ~/.agent-graph/images/   Pictures from `snapshot`
  (%USERPROFILE%\\.agent-graph on Windows)

Settings (environment variables):
  AGENT_GRAPH_HOME=<dir>          Keep data somewhere else
  AGENT_GRAPH_CAPTURE_BODIES=1    Also record message bodies and final agent messages
  AGENT_GRAPH_RAW=1               Also keep raw hook payloads, for building adapters
  AGENT_GRAPH_NOW=<RFC 3339>      Read logs as if it were this time

Run `agent-graph <command> --help` for more on a command.";

const INSTALL_EXAMPLES: &str = "\
Examples:
  agent-graph install claude-code                   Hooks and /agent-graph, for every project
  agent-graph install claude-code --scope project   Just this project (in .claude/, committed)
  agent-graph install claude-code --dry-run         Show what would change
  agent-graph install codex                         $agent-graph in Codex (its sessions aren't recorded yet)
  agent-graph install gemini                        /agent-graph in Gemini CLI
  agent-graph install cursor                        /agent-graph in Cursor";

const SNAPSHOT_EXAMPLES: &str = "\
Examples:
  agent-graph snapshot                       This session (or the latest one here), as a PNG
  agent-graph snapshot --all --theme dark    Every session from the last day
  agent-graph snapshot --out graph.svg       As SVG
  agent-graph snapshot --json                The whole graph, as JSON";

const TAIL_EXAMPLES: &str = "\
Keys: q quits, a shows or hides sessions older than a day.

Examples:
  agent-graph tail                     Every session from the last day
  agent-graph tail --session current   Just the session this runs in
  agent-graph tail | tee graph.log     Not a terminal: prints a new frame on each change";

const WATCH_REMOTE_EXAMPLES: &str = "\
Examples:
  agent-graph watch-remote                                          Share every session, live
  agent-graph watch-remote --session current                        Just the session this runs in
  agent-graph watch-remote --password=s3cret --save-default-password   And use it from now on
  agent-graph watch-remote --password= --save-default-password         Forget the saved password
  agent-graph watch-remote --url=http://localhost:3000              Share to a local copy of the site";

#[derive(Parser)]
#[command(
    name = "agent-graph",
    version,
    about = ABOUT,
    long_about = LONG_ABOUT,
    after_help = AFTER_HELP,
    after_long_help = AFTER_LONG_HELP
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Record one hook event read from stdin. Provider hooks run this; it
    /// always exits 0 and never prints.
    #[command(hide = true)]
    Emit {
        #[arg(long)]
        provider: String,
    },
    /// Set up a coding agent: hooks that record its sessions, and an
    /// /agent-graph command
    ///
    /// For Claude Code this adds the hooks that record its sessions, and an
    /// /agent-graph command (a skill) you can run in any session to see the
    /// graph. For Codex, Gemini CLI and Cursor it adds just the command for
    /// now. Everything else in their settings is kept, and settings files are
    /// backed up first.
    #[command(display_order = 1, after_help = INSTALL_EXAMPLES)]
    Install {
        provider: Provider,
        #[arg(long, value_enum, default_value_t = ScopeArg::User)]
        scope: ScopeArg,
        /// Show what would change without writing anything.
        #[arg(long)]
        dry_run: bool,
        /// Don't ask for confirmation.
        #[arg(long, short)]
        yes: bool,
        /// The hook command to register. Defaults to this executable.
        #[arg(long)]
        command: Option<String>,
        /// Don't add the `/agent-graph` command.
        #[arg(long)]
        no_slash_command: bool,
    },
    /// Remove what `install` added
    #[command(display_order = 2)]
    Uninstall {
        provider: Provider,
        #[arg(long, value_enum, default_value_t = ScopeArg::User)]
        scope: ScopeArg,
        #[arg(long)]
        dry_run: bool,
        #[arg(long, short)]
        yes: bool,
    },
    /// Print the graph once, as a text tree
    #[command(display_order = 5)]
    Tree {
        /// Include sessions with no activity in the last 24 hours.
        #[arg(long)]
        all: bool,
        #[arg(long, default_value_t = 30)]
        stale_minutes: u64,
    },
    /// Save a picture of a session, sized for a phone, and print its path
    ///
    /// Made for sharing: an agent can run this and send you the image, so
    /// you can check on your agents from your phone. With --json it gives
    /// the whole graph as data instead.
    #[command(display_order = 6, after_help = SNAPSHOT_EXAMPLES)]
    Snapshot {
        /// The session to draw: its id, the first few characters of it, or
        /// `current`. Defaults to (and `current` means) the Claude Code session
        /// this runs inside, then the most recent session in this folder, then
        /// the most recent overall.
        #[arg(long)]
        session: Option<String>,
        /// Draw every session active in the last 24 hours instead.
        #[arg(long, conflicts_with = "session")]
        all: bool,
        /// Where to write it. The format follows the extension: .png, .svg
        /// or .json. Defaults to a new PNG in the data directory's images
        /// folder (or, with --json, to standard output).
        #[arg(long, short)]
        out: Option<PathBuf>,
        /// The whole graph as JSON, instead of a picture.
        #[arg(long, conflicts_with_all = ["session", "all"])]
        json: bool,
        #[arg(long, value_enum, default_value_t = ThemeArg::Light)]
        theme: ThemeArg,
        /// Flag unfinished nodes with no events for this many minutes.
        #[arg(long, default_value_t = 30)]
        stale_minutes: u64,
    },
    /// Watch the graph live in this terminal
    ///
    /// Takes over the terminal like `top` and redraws as events arrive, with
    /// a summary of what needs you, what looks stuck and any deadlocks.
    #[command(display_order = 3, after_help = TAIL_EXAMPLES)]
    Tail {
        /// Follow one session: its id, or the first few characters of it.
        #[arg(long)]
        session: Option<String>,
        /// Include sessions with no activity in the last 24 hours.
        #[arg(long)]
        all: bool,
        /// Draw the tree with plain ASCII characters.
        #[arg(long)]
        ascii: bool,
        #[arg(long, default_value_t = 30)]
        stale_minutes: u64,
    },
    /// Share the graph live on the web, and print its link
    ///
    /// Sends the event log to the Agent Graph site, then each new event as
    /// it's recorded, until you stop it (Ctrl+C). The link is printed first,
    /// straight away. Anyone with it can view the graph unless you set a
    /// password. Only this run can add to the shared log.
    #[command(display_order = 7, after_help = WATCH_REMOTE_EXAMPLES)]
    WatchRemote {
        /// The site to share on.
        #[arg(long, default_value = crate::remote::DEFAULT_URL)]
        url: String,
        /// Viewers must enter this to see the log. `--password=` means none,
        /// even if a default is saved.
        #[arg(long)]
        password: Option<String>,
        /// Also save --password as the default for future runs (an empty
        /// --password= clears the saved default).
        #[arg(long)]
        save_default_password: bool,
        /// Share just this session: its id, the first few characters of it, or
        /// `current` (the one this runs inside, as for `snapshot`).
        #[arg(long)]
        session: Option<String>,
    },
    /// Watch the graph live in your browser, with a timeline to step through
    ///
    /// Serves http://localhost:7777 on this machine only. The page updates as
    /// events arrive, and its timeline steps back through every event in a
    /// session.
    #[command(display_order = 4)]
    View {
        #[arg(long, default_value_t = 7777)]
        port: u16,
        /// Open it in your browser.
        #[arg(long)]
        open: bool,
        #[arg(long, default_value_t = 30)]
        stale_minutes: u64,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum Provider {
    ClaudeCode,
    Codex,
    Gemini,
    Cursor,
}

impl From<Provider> for Client {
    fn from(p: Provider) -> Client {
        match p {
            Provider::ClaudeCode => Client::ClaudeCode,
            Provider::Codex => Client::Codex,
            Provider::Gemini => Client::Gemini,
            Provider::Cursor => Client::Cursor,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum ThemeArg {
    Light,
    Dark,
}

#[derive(Clone, Copy, ValueEnum)]
enum ScopeArg {
    User,
    Project,
    Local,
}

impl From<ScopeArg> for Scope {
    fn from(s: ScopeArg) -> Scope {
        match s {
            ScopeArg::User => Scope::User,
            ScopeArg::Project => Scope::Project,
            ScopeArg::Local => Scope::Local,
        }
    }
}

pub fn run() -> ExitCode {
    if std::env::args_os().len() <= 1 {
        use clap::CommandFactory;
        let _ = Cli::command().print_help();
        return ExitCode::SUCCESS;
    }
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Emit { .. } => unreachable!("main handles emit before parsing"),
        Command::Install {
            provider,
            scope,
            dry_run,
            yes,
            command,
            no_slash_command,
        } => install_cmd(
            provider.into(),
            scope.into(),
            InstallOptions {
                dry_run,
                yes,
                hook_command: command,
                slash_command: !no_slash_command,
                add: true,
            },
        ),
        Command::Uninstall {
            provider,
            scope,
            dry_run,
            yes,
        } => install_cmd(
            provider.into(),
            scope.into(),
            InstallOptions {
                dry_run,
                yes,
                hook_command: None,
                slash_command: true,
                add: false,
            },
        ),
        Command::Snapshot {
            session,
            all,
            out,
            json,
            theme,
            stale_minutes,
        } => snapshot_cmd(session, all, out, json, theme, stale_minutes),
        Command::Tree { all, stale_minutes } => tree_cmd(all, stale_minutes),
        Command::Tail {
            session,
            all,
            ascii,
            stale_minutes,
        } => {
            let root = paths::data_dir().ok_or("can't find your home directory".to_string());
            root.and_then(|root| {
                crate::live::run(
                    &paths::events_dir(&root),
                    crate::live::Options {
                        session,
                        all,
                        ascii,
                        stale_after: Duration::from_secs(stale_minutes * 60),
                    },
                )
            })
        }
        Command::WatchRemote {
            url,
            password,
            save_default_password,
            session,
        } => paths::data_dir()
            .ok_or_else(|| "can't find your home directory".to_string())
            .and_then(|root| {
                crate::remote::run(
                    &root,
                    crate::remote::Options {
                        url,
                        password,
                        save_default_password,
                        session,
                    },
                )
            }),
        Command::View {
            port,
            open,
            stale_minutes,
        } => view_cmd(port, open, stale_minutes),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("agent-graph: {e}");
            ExitCode::FAILURE
        }
    }
}

struct InstallOptions {
    dry_run: bool,
    yes: bool,
    hook_command: Option<String>,
    slash_command: bool,
    /// Install, or (false) uninstall.
    add: bool,
}

/// One file `install` or `uninstall` would write or remove.
struct Change {
    path: PathBuf,
    /// The new contents, or None to remove the file.
    contents: Option<String>,
    /// Lines describing the change, shown before asking.
    summary: Vec<String>,
    /// Keep a copy of the old file first (settings files).
    backup: bool,
}

fn install_cmd(client: Client, scope: Scope, opts: InstallOptions) -> Result<(), String> {
    let cwd =
        std::env::current_dir().map_err(|e| format!("can't read the current directory: {e}"))?;
    let mut changes = Vec::new();
    let mut notes = Vec::new();

    if client == Client::ClaudeCode {
        if let Some(change) = hooks_change(scope, &cwd, &opts)? {
            changes.push(change);
        }
    } else if opts.add {
        notes.push(format!(
            "{} sessions aren't recorded yet: only Claude Code has hooks so far. \
             The command shows the sessions that are.",
            slash::client_name(client)
        ));
    }

    if opts.slash_command {
        let file = slash::command_file(client, scope, &cwd)?;
        let existing = std::fs::read_to_string(&file.path).ok();
        let ours = existing.as_deref().is_some_and(slash::is_ours);
        match (opts.add, existing.as_deref()) {
            (true, Some(_)) if !ours => notes.push(format!(
                "Left {} alone: it isn't one Agent Graph wrote.",
                file.path.display()
            )),
            (true, Some(text)) if text == file.contents => {}
            (true, _) => changes.push(Change {
                summary: vec![
                    format!("Command file: {}", file.path.display()),
                    format!("Adds the {} command.", file.invoke),
                ],
                path: file.path,
                contents: Some(file.contents),
                backup: false,
            }),
            (false, Some(_)) if ours => changes.push(Change {
                summary: vec![format!(
                    "Removes the {} command: {}",
                    file.invoke,
                    file.path.display()
                )],
                path: file.path,
                contents: None,
                backup: false,
            }),
            (false, _) => {}
        }
    }

    for note in &notes {
        println!("{note}");
    }
    if changes.is_empty() {
        println!(
            "Nothing to do: {}.",
            if opts.add {
                "Agent Graph is already set up"
            } else {
                "nothing of Agent Graph's is installed"
            }
        );
        return Ok(());
    }
    for change in &changes {
        for line in &change.summary {
            println!("{line}");
        }
    }
    if opts.dry_run {
        for change in &changes {
            match &change.contents {
                Some(text) => println!("\n--- {} (not written) ---\n{text}", change.path.display()),
                None => println!("\n--- {} (not removed) ---", change.path.display()),
            }
        }
        return Ok(());
    }
    if !opts.yes && !confirm("Make these changes?")? {
        println!("Nothing changed.");
        return Ok(());
    }

    for change in &changes {
        apply(change)?;
    }
    if opts.add {
        if client == Client::ClaudeCode {
            let data = paths::data_dir()
                .map(|d| d.display().to_string())
                .unwrap_or_default();
            println!("Installed. New Claude Code sessions will be recorded in {data}.");
        } else {
            println!("Installed.");
        }
    } else {
        println!("Uninstalled.");
    }
    Ok(())
}

/// The change to Claude Code's settings for the hooks, if any.
fn hooks_change(scope: Scope, cwd: &Path, opts: &InstallOptions) -> Result<Option<Change>, String> {
    let path = install::claude_settings_path(scope, cwd).ok_or("can't find your home directory")?;
    let existed = path.exists();
    let before: Value = if existed {
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("reading {}: {e}", path.display()))?;
        serde_json::from_str(&text).map_err(|e| {
            format!(
                "{} isn't valid JSON, so it wasn't changed: {e}",
                path.display()
            )
        })?
    } else {
        serde_json::json!({})
    };

    let mut after = before.clone();
    let command = match &opts.hook_command {
        Some(c) => c.clone(),
        None => install::default_command("claude-code")?,
    };
    if opts.add {
        install::install_claude_code(&mut after, &command)?;
    } else {
        install::uninstall_claude_code(&mut after)?;
    }
    if after == before {
        return Ok(None);
    }

    let mut summary = vec![format!("Settings file: {}", path.display())];
    if opts.add {
        summary.push(format!("Hook command:  {command}"));
        summary.push(format!(
            "Adds hooks for: {}",
            install::our_events(&after).join(", ")
        ));
        if !install::our_events(&before).is_empty() {
            summary.push("(Replaces the Agent Graph hooks already there.)".into());
        }
    } else {
        summary.push(format!(
            "Removes hooks for: {}",
            install::our_events(&before).join(", ")
        ));
    }
    Ok(Some(Change {
        path,
        contents: Some(serde_json::to_string_pretty(&after).expect("serializable") + "\n"),
        summary,
        backup: existed,
    }))
}

fn apply(change: &Change) -> Result<(), String> {
    let path = &change.path;
    let Some(text) = &change.contents else {
        std::fs::remove_file(path).map_err(|e| format!("removing {}: {e}", path.display()))?;
        // Don't leave empty folders behind: a skill's own folder, then the
        // skills/commands folder if nothing else is in it (remove_dir only
        // removes empty folders).
        let mut dir = path.parent();
        if let Some(d) = dir.filter(|d| slash::is_own_folder(d)) {
            let _ = std::fs::remove_dir(d);
            dir = d.parent();
        }
        if let Some(d) = dir.filter(|d| {
            d.file_name()
                .is_some_and(|n| n == "skills" || n == "commands")
        }) {
            let _ = std::fs::remove_dir(d);
        }
        return Ok(());
    };
    if change.backup {
        let backup = path.with_extension("json.agent-graph.bak");
        std::fs::copy(path, &backup)
            .map_err(|e| format!("backing up to {}: {e}", backup.display()))?;
        println!("Backed up the old file to {}", backup.display());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    }
    store::write_atomic(path, text.as_bytes())
        .map_err(|e| format!("writing {}: {e}", path.display()))
}

fn confirm(question: &str) -> Result<bool, String> {
    if !io::stdin().is_terminal() {
        return Err(
            "not running in a terminal; re-run with --yes to confirm, or --dry-run to preview"
                .into(),
        );
    }
    print!("{question} [y/N] ");
    io::stdout().flush().ok();
    let mut answer = String::new();
    io::stdin()
        .lock()
        .read_line(&mut answer)
        .map_err(|e| e.to_string())?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn load_graph(stale_minutes: u64) -> Result<(Graph, std::path::PathBuf), String> {
    let root = paths::data_dir().ok_or("can't find your home directory")?;
    let loaded = store::load_events(&paths::events_dir(&root))
        .map_err(|e| format!("reading events in {}: {e}", root.display()))?;
    if loaded.skipped_lines > 0 {
        eprintln!(
            "agent-graph: skipped {} unreadable event line(s)",
            loaded.skipped_lines
        );
    }
    let opts = reducer::Options {
        now: crate::clock::now(),
        stale_after: Duration::from_secs(stale_minutes * 60),
    };
    Ok((reducer::reduce(loaded.events, &opts), root))
}

fn tree_cmd(all: bool, stale_minutes: u64) -> Result<(), String> {
    let (graph, root) = load_graph(stale_minutes)?;
    let cutoff = crate::clock::now() - Duration::from_secs(24 * 60 * 60);
    let roots: Vec<String> = graph
        .roots
        .iter()
        .filter(|id| {
            all || humantime::parse_rfc3339_weak(&graph.nodes[*id].last_event_at)
                .is_ok_and(|t| t >= cutoff)
        })
        .cloned()
        .collect();
    if roots.is_empty() {
        let hint = if graph.roots.is_empty() {
            "Run `agent-graph install claude-code`, then start a new Claude Code session."
        } else {
            "Nothing in the last 24 hours; use --all to see older sessions."
        };
        println!("No sessions in {}. {hint}", display_dir(&root));
        return Ok(());
    }
    print!("{}", render::tree(&graph, &roots));
    Ok(())
}

fn snapshot_cmd(
    session: Option<String>,
    all: bool,
    out: Option<PathBuf>,
    json: bool,
    theme: ThemeArg,
    stale_minutes: u64,
) -> Result<(), String> {
    let (graph, root) = load_graph(stale_minutes)?;
    let ext = out
        .as_ref()
        .and_then(|p| p.extension())
        .map(|e| e.to_string_lossy().to_ascii_lowercase());

    if json || ext.as_deref() == Some("json") {
        let text = serde_json::to_string_pretty(&graph).expect("serializable") + "\n";
        match out {
            Some(path) => {
                store::write_atomic(&path, text.as_bytes())
                    .map_err(|e| format!("writing {}: {e}", path.display()))?;
                println!("{}", path.display());
            }
            None => print!("{text}"),
        }
        return Ok(());
    }

    if graph.roots.is_empty() {
        return Err(format!(
            "no sessions recorded yet in {}. Run `agent-graph install claude-code`, then start a new session.",
            root.display()
        ));
    }
    let cwd = std::env::current_dir().ok();
    let roots = pick_roots(&graph, session.as_deref(), all, cwd.as_deref())?;
    let theme = match theme {
        ThemeArg::Light => crate::image::Theme::Light,
        ThemeArg::Dark => crate::image::Theme::Dark,
    };
    let svg = crate::image::svg(
        &graph,
        &roots,
        &crate::image::Options {
            theme,
            as_of: crate::clock::now(),
        },
    );

    let path = match out {
        Some(path) => path,
        None => {
            let dir = root.join("images");
            store::ensure_dir(&dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
            let stamp = humantime::format_rfc3339_seconds(SystemTime::now())
                .to_string()
                .replace(['-', ':'], "")
                .replace('T', "-")
                .trim_end_matches('Z')
                .to_string();
            dir.join(format!("agent-graph-{stamp}.png"))
        }
    };
    let bytes = if ext.as_deref() == Some("svg") {
        svg.into_bytes()
    } else {
        crate::image::png(&svg)?
    };
    std::fs::write(&path, bytes).map_err(|e| format!("writing {}: {e}", path.display()))?;
    // Only the path goes to stdout, so scripts and agents can use it directly.
    println!("{}", path.display());
    Ok(())
}

/// Which trees to draw. See the `Snapshot` command's docs for the order.
pub fn pick_roots(
    graph: &Graph,
    session: Option<&str>,
    all: bool,
    cwd: Option<&Path>,
) -> Result<Vec<String>, String> {
    if all {
        let cutoff = crate::clock::now() - Duration::from_secs(24 * 60 * 60);
        let recent: Vec<String> = graph
            .roots
            .iter()
            .filter(|id| {
                humantime::parse_rfc3339_weak(&graph.nodes[*id].last_event_at)
                    .is_ok_and(|t| t >= cutoff)
            })
            .cloned()
            .collect();
        return Ok(if recent.is_empty() {
            graph.roots.iter().take(1).cloned().collect()
        } else {
            recent
        });
    }
    // "current" means the default below: the session this runs in.
    if let Some(want) = session.filter(|s| *s != "current") {
        return find_node(graph, want).map(|id| vec![id]);
    }
    // Inside Claude Code, commands can see the session they run in.
    if let Ok(id) = std::env::var("CLAUDE_CODE_SESSION_ID") {
        let node = format!("claude-code:{id}");
        if graph.nodes.contains_key(&node) {
            return Ok(vec![node]);
        }
    }
    if let Some(cwd) = cwd {
        let here = graph.roots.iter().find(|id| {
            graph.nodes[*id]
                .cwd
                .as_deref()
                .is_some_and(|c| Path::new(c) == cwd)
        });
        if let Some(id) = here {
            return Ok(vec![id.clone()]);
        }
    }
    Ok(graph.roots.iter().take(1).cloned().collect())
}

/// Finds a node by its full id, its own id (the session id for a session,
/// the agent id for an agent), or a unique prefix of either.
pub(crate) fn find_node(graph: &Graph, want: &str) -> Result<String, String> {
    if graph.nodes.contains_key(want) {
        return Ok(want.to_string());
    }
    let own_id = |id: &str| id.rsplit(['/', ':']).next().unwrap_or(id).to_string();
    let matches: Vec<&String> = graph
        .nodes
        .keys()
        .filter(|id| id.starts_with(want) || own_id(id).starts_with(want))
        .collect();
    match matches.as_slice() {
        [one] => Ok((*one).clone()),
        [] => Err(format!("no session matches {want:?}")),
        many => Err(format!(
            "{want:?} matches {} sessions or agents; use more of the id",
            many.len()
        )),
    }
}

fn view_cmd(port: u16, open: bool, stale_minutes: u64) -> Result<(), String> {
    let root = paths::data_dir().ok_or("can't find your home directory")?;
    crate::view::run(
        &paths::events_dir(&root),
        crate::view::Options {
            port,
            open,
            stale_after: Duration::from_secs(stale_minutes * 60),
        },
    )
}

fn display_dir(path: &Path) -> String {
    path.display().to_string()
}
