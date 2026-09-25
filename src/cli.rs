//! Every subcommand except `emit`, which `main` handles before clap runs.

use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, SystemTime};

use clap::{Parser, Subcommand, ValueEnum};
use serde_json::Value;

use crate::install::{self, Scope};
use crate::reducer::{self, Graph};
use crate::{paths, render, store};

#[derive(Parser)]
#[command(
    name = "agent-graph",
    version,
    about = "Records a live graph of AI coding agent sessions"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Record one hook event read from stdin. Provider hooks run this; it
    /// always exits 0 and never prints.
    Emit {
        #[arg(long)]
        provider: String,
    },
    /// Add Agent Graph's hooks to a provider's settings.
    Install {
        provider: Provider,
        #[arg(long, value_enum, default_value_t = ScopeArg::User)]
        scope: ScopeArg,
        /// Show the resulting settings file without writing it.
        #[arg(long)]
        dry_run: bool,
        /// Don't ask for confirmation.
        #[arg(long, short)]
        yes: bool,
        /// The hook command to register. Defaults to this executable.
        #[arg(long)]
        command: Option<String>,
    },
    /// Remove Agent Graph's hooks from a provider's settings.
    Uninstall {
        provider: Provider,
        #[arg(long, value_enum, default_value_t = ScopeArg::User)]
        scope: ScopeArg,
        #[arg(long)]
        dry_run: bool,
        #[arg(long, short)]
        yes: bool,
    },
    /// Print the current graph as a tree.
    Tree {
        /// Include sessions with no activity in the last 24 hours.
        #[arg(long)]
        all: bool,
        #[arg(long, default_value_t = 30)]
        stale_minutes: u64,
    },
    /// Save a picture of a session's graph and print where it went.
    ///
    /// Made for sharing: an agent can run this and send you the image, so
    /// you can check on your agents from your phone. With --json it gives
    /// the whole graph as data instead.
    Snapshot {
        /// The session to draw: its id, or the first few characters of it.
        /// Defaults to the Claude Code session this runs inside, then the
        /// most recent session in this folder, then the most recent overall.
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
    /// Watch the graph live in this terminal, redrawn as events arrive.
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
    /// Share the graph live on the Agent Graph site, and print its link.
    ///
    /// Sends the event log, then each new event as it's recorded, until you
    /// stop it. Anyone with the link can view it unless you set a password.
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
        /// Share just this session (its id or the first few characters).
        #[arg(long)]
        session: Option<String>,
    },
    /// Serve a live web view of the graph on this machine.
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
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Emit { .. } => unreachable!("main handles emit before parsing"),
        Command::Install {
            provider: Provider::ClaudeCode,
            scope,
            dry_run,
            yes,
            command,
        } => install_cmd(scope.into(), dry_run, yes, command, true),
        Command::Uninstall {
            provider: Provider::ClaudeCode,
            scope,
            dry_run,
            yes,
        } => install_cmd(scope.into(), dry_run, yes, None, false),
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

fn install_cmd(
    scope: Scope,
    dry_run: bool,
    yes: bool,
    command: Option<String>,
    add: bool,
) -> Result<(), String> {
    let cwd =
        std::env::current_dir().map_err(|e| format!("can't read the current directory: {e}"))?;
    let path =
        install::claude_settings_path(scope, &cwd).ok_or("can't find your home directory")?;
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
    let command = match command {
        Some(c) => c,
        None => install::default_command("claude-code")?,
    };
    if add {
        install::install_claude_code(&mut after, &command)?;
    } else {
        install::uninstall_claude_code(&mut after)?;
    }
    if after == before {
        println!(
            "{} already {}; nothing to do.",
            path.display(),
            if add {
                "has Agent Graph's hooks"
            } else {
                "has no Agent Graph hooks"
            }
        );
        return Ok(());
    }

    println!("Settings file: {}", path.display());
    if add {
        println!("Hook command:  {command}");
        println!("Adds hooks for: {}", install::our_events(&after).join(", "));
        if !install::our_events(&before).is_empty() {
            println!("(Replaces the Agent Graph hooks already there.)");
        }
    } else {
        println!(
            "Removes hooks for: {}",
            install::our_events(&before).join(", ")
        );
    }
    let text = serde_json::to_string_pretty(&after).expect("serializable") + "\n";
    if dry_run {
        println!("\n--- {} (not written) ---\n{text}", path.display());
        return Ok(());
    }
    if !yes && !confirm("Write these changes?")? {
        println!("Nothing written.");
        return Ok(());
    }

    if existed {
        let backup = path.with_extension("json.agent-graph.bak");
        std::fs::copy(&path, &backup)
            .map_err(|e| format!("backing up to {}: {e}", backup.display()))?;
        println!("Backed up the old file to {}", backup.display());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    }
    store::write_atomic(&path, text.as_bytes())
        .map_err(|e| format!("writing {}: {e}", path.display()))?;
    if add {
        let data = paths::data_dir()
            .map(|d| d.display().to_string())
            .unwrap_or_default();
        println!("Installed. New Claude Code sessions will be recorded in {data}.");
    } else {
        println!("Uninstalled.");
    }
    Ok(())
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
    if let Some(want) = session {
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
