//! Every subcommand except `emit`, which `main` handles before clap runs.

use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, SystemTime};

use clap::{Arg, ArgAction, CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum};
use serde_json::Value;

use crate::help;
use crate::install::{self, Scope};
use crate::reducer::{self, Graph};
use crate::slash::{self, Client};
use crate::{paths, render, store};

// All help text comes from docs/cli-help.json (compiled in by build.rs), which
// the site's /docs page renders too. Don't add doc comments here: they'd
// override it. The tests check the file covers every command and option.

#[derive(Parser)]
#[command(
    name = "agent-graph",
    version,
    about = help::SUMMARY,
    long_about = help::LONG_ABOUT,
    after_help = help::AFTER_HELP,
    after_long_help = help::AFTER_LONG_HELP
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    // Run by the hooks, with a hook's JSON on stdin; always exits 0 and never
    // prints. Handled in main before argument parsing.
    #[command(hide = true)]
    Emit {
        #[arg(long)]
        provider: String,
    },
    #[command(
        display_order = 1,
        about = help::install::SUMMARY,
        long_about = help::install::DESCRIPTION,
        after_help = help::install::EXAMPLES
    )]
    Install {
        #[arg(help = help::install::opt::PROVIDER)]
        provider: Provider,
        #[arg(long, value_enum, default_value_t = ScopeArg::User, help = help::install::opt::SCOPE)]
        scope: ScopeArg,
        #[arg(long, help = help::install::opt::DRY_RUN)]
        dry_run: bool,
        #[arg(long, short, help = help::install::opt::YES)]
        yes: bool,
        #[arg(long, help = help::install::opt::COMMAND)]
        command: Option<String>,
        #[arg(long, help = help::install::opt::NO_SLASH_COMMAND)]
        no_slash_command: bool,
    },
    #[command(
        display_order = 2,
        about = help::uninstall::SUMMARY,
        long_about = help::uninstall::DESCRIPTION,
        after_help = help::uninstall::EXAMPLES
    )]
    Uninstall {
        #[arg(help = help::uninstall::opt::PROVIDER)]
        provider: Provider,
        #[arg(long, value_enum, default_value_t = ScopeArg::User, help = help::uninstall::opt::SCOPE)]
        scope: ScopeArg,
        #[arg(long, help = help::uninstall::opt::DRY_RUN)]
        dry_run: bool,
        #[arg(long, short, help = help::uninstall::opt::YES)]
        yes: bool,
    },
    #[command(
        display_order = 5,
        about = help::tree::SUMMARY,
        long_about = help::tree::DESCRIPTION,
        after_help = help::tree::EXAMPLES
    )]
    Tree {
        #[arg(long, help = help::tree::opt::ALL)]
        all: bool,
        #[arg(long, default_value_t = 30, help = help::tree::opt::STALE_MINUTES)]
        stale_minutes: u64,
    },
    #[command(
        display_order = 6,
        about = help::snapshot::SUMMARY,
        long_about = help::snapshot::DESCRIPTION,
        after_help = help::snapshot::EXAMPLES
    )]
    Snapshot {
        #[arg(long, help = help::snapshot::opt::SESSION)]
        session: Option<String>,
        #[arg(long, conflicts_with = "session", help = help::snapshot::opt::ALL)]
        all: bool,
        #[arg(long, short, help = help::snapshot::opt::OUT)]
        out: Option<PathBuf>,
        #[arg(long, requires = "out", help = help::snapshot::opt::FORCE)]
        force: bool,
        #[arg(long, conflicts_with_all = ["session", "all"], help = help::snapshot::opt::JSON)]
        json: bool,
        #[arg(long, value_enum, default_value_t = ThemeArg::Light, help = help::snapshot::opt::THEME)]
        theme: ThemeArg,
        #[arg(long, default_value_t = 30, help = help::snapshot::opt::STALE_MINUTES)]
        stale_minutes: u64,
    },
    #[command(
        display_order = 3,
        about = help::tail::SUMMARY,
        long_about = help::tail::DESCRIPTION,
        after_help = help::tail::EXAMPLES
    )]
    Tail {
        #[arg(long, help = help::tail::opt::SESSION)]
        session: Option<String>,
        #[arg(long, help = help::tail::opt::ALL)]
        all: bool,
        #[arg(long, help = help::tail::opt::ASCII)]
        ascii: bool,
        #[arg(long, default_value_t = 30, help = help::tail::opt::STALE_MINUTES)]
        stale_minutes: u64,
    },
    #[command(
        display_order = 7,
        about = help::watch_remote::SUMMARY,
        long_about = help::watch_remote::DESCRIPTION,
        after_help = help::watch_remote::EXAMPLES
    )]
    WatchRemote {
        #[arg(long, default_value = crate::remote::DEFAULT_URL, help = help::watch_remote::opt::URL)]
        url: String,
        #[arg(long, help = help::watch_remote::opt::PASSWORD)]
        password: Option<String>,
        #[arg(long, help = help::watch_remote::opt::SAVE_DEFAULT_PASSWORD)]
        save_default_password: bool,
        #[arg(long, help = help::watch_remote::opt::SESSION)]
        session: Option<String>,
    },
    #[command(
        display_order = 4,
        about = help::view::SUMMARY,
        long_about = help::view::DESCRIPTION,
        after_help = help::view::EXAMPLES
    )]
    View {
        #[arg(long, default_value_t = 7777, help = help::view::opt::PORT)]
        port: u16,
        #[arg(long, help = help::view::opt::OPEN)]
        open: bool,
        #[arg(long, default_value_t = 30, help = help::view::opt::STALE_MINUTES)]
        stale_minutes: u64,
    },
    #[command(
        display_order = 8,
        about = help::run::SUMMARY,
        long_about = help::run::DESCRIPTION,
        after_help = help::run::EXAMPLES
    )]
    Run {
        #[arg(long, help = help::run::opt::NAME)]
        name: Option<String>,
        #[arg(
            required = true,
            trailing_var_arg = true,
            allow_hyphen_values = true,
            value_name = "COMMAND",
            help = help::run::opt::COMMAND
        )]
        command: Vec<std::ffi::OsString>,
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
    // Every command's -h and --help show its full explanation and examples.
    // (At the top level, -h stays a short summary.)
    let command = Cli::command().mut_subcommands(|sub| {
        sub.disable_help_flag(true).arg(
            Arg::new("help")
                .short('h')
                .long("help")
                .action(ArgAction::HelpLong)
                .help("Explain this command in full, with examples"),
        )
    });
    if std::env::args_os().len() <= 1 {
        let _ = command.clone().print_help();
        return ExitCode::SUCCESS;
    }
    let cli = Cli::from_arg_matches(&command.get_matches()).unwrap_or_else(|e| e.exit());
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
            force,
            json,
            theme,
            stale_minutes,
        } => snapshot_cmd(
            session,
            all,
            out.map(|path| Out { path, force }),
            json,
            theme,
            stale_minutes,
        ),
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
        Command::Run { name, command } => {
            return crate::run::run(crate::run::Options { name, command });
        }
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
    /// Where to keep a copy of the old file first (settings files).
    backup: Option<PathBuf>,
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
        inside_project(scope, &cwd, &file.path)?;
        // Only a missing file is absent. One that can't be read may be the
        // user's own, so it's left alone; bytes that aren't UTF-8 are read
        // as they are (it's still theirs unless it has our marker).
        let existing = match std::fs::read(&file.path) {
            Ok(bytes) => Some(Some(bytes)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Some(None),
            Err(e) => {
                notes.push(format!(
                    "Left {} alone: can't read it ({e}).",
                    file.path.display()
                ));
                None
            }
        };
        if let Some(existing) = existing {
            let ours = existing.as_deref().is_some_and(slash::bytes_are_ours);
            match (opts.add, existing.as_deref()) {
                (true, Some(_)) if !ours => notes.push(format!(
                    "Left {} alone: it isn't one Agent Graph wrote.",
                    file.path.display()
                )),
                (true, Some(bytes)) if bytes == file.contents.as_bytes() => {}
                (true, _) => changes.push(Change {
                    summary: vec![
                        format!("Command file: {}", file.path.display()),
                        format!("Adds the {} command.", file.invoke),
                    ],
                    path: file.path,
                    contents: Some(file.contents),
                    backup: None,
                }),
                (false, Some(_)) if ours => changes.push(Change {
                    summary: vec![format!(
                        "Removes the {} command: {}",
                        file.invoke,
                        file.path.display()
                    )],
                    path: file.path,
                    contents: None,
                    backup: None,
                }),
                (false, _) => {}
            }
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

/// A project can ship anything, including links out of itself. For project
/// and local scope, refuses a path that is, or goes through, a link in it.
fn inside_project(scope: Scope, cwd: &Path, path: &Path) -> Result<(), String> {
    if scope == Scope::User {
        return Ok(());
    }
    store::refuse_links(cwd, path)
        .map_err(|e| format!("{e}; Agent Graph won't read or write through links in a project"))
}

/// The change to Claude Code's settings for the hooks, if any.
fn hooks_change(scope: Scope, cwd: &Path, opts: &InstallOptions) -> Result<Option<Change>, String> {
    let link = install::claude_settings_path(scope, cwd).ok_or("can't find your home directory")?;
    inside_project(scope, cwd, &link)?;
    // Your own settings file can be a link (into a dotfiles repo, say): the
    // change goes to the file it points to, and the backup stays here.
    // (A project's links were refused above.)
    let is_link = std::fs::symlink_metadata(&link).is_ok_and(|m| m.file_type().is_symlink());
    let path = if is_link {
        std::fs::canonicalize(&link)
            .map_err(|e| format!("{} is a link that can't be followed: {e}", link.display()))?
    } else {
        link.clone()
    };
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
    let this_on_path = install::this_is_on_path();
    let command = match &opts.hook_command {
        Some(c) => c.clone(),
        // Shared settings can't name this machine's copy. Everywhere else
        // the full path is used: hooks run with Claude Code's own PATH,
        // which (started from a launcher or a scheduler) may not have it.
        None if scope == Scope::Project => install::path_command("claude-code"),
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

    let mut summary = vec![if is_link {
        format!(
            "Settings file: {} (a link to {})",
            link.display(),
            path.display()
        )
    } else {
        format!("Settings file: {}", path.display())
    }];
    if opts.add {
        summary.push(format!("Hook command:  {command}"));
        summary.push(format!(
            "Adds hooks for: {}",
            install::our_events(&after).join(", ")
        ));
        if !install::our_events(&before).is_empty() {
            summary.push("(Replaces the Agent Graph hooks already there.)".into());
        }
        if scope == Scope::Project && opts.hook_command.is_none() {
            summary.push(
                "Project settings are shared, so the hooks run `agent-graph` from PATH: \
                 everyone who uses the project needs it installed."
                    .into(),
            );
        }
        if command == install::path_command("claude-code") {
            match this_on_path {
                None => summary.push(
                    "Note: `agent-graph` isn't on your PATH, so these hooks won't run for you \
                     until it is (cargo install --path .)."
                        .into(),
                ),
                Some(false) => summary.push(
                    "Note: the `agent-graph` on your PATH is a different copy from this one; \
                     the hooks will run that."
                        .into(),
                ),
                Some(true) => {}
            }
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
        backup: existed.then(|| link.with_extension("json.agent-graph.bak")),
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
    if let Some(backup) = &change.backup {
        // Written like any other file, so a link left at the backup's name
        // is replaced rather than written through. It holds what the
        // original does, so it gets the original's permissions.
        std::fs::read(path)
            .and_then(|original| {
                let permissions = std::fs::metadata(path)?.permissions();
                store::write_atomic_with(backup, &original, Some(permissions))
            })
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

/// Where `snapshot --out` writes, and whether it may replace a file there.
struct Out {
    path: PathBuf,
    force: bool,
}

impl Out {
    /// Writes a new file, or with `--force`, replaces one. Refusing by
    /// default means `--out` can't be used to overwrite someone's files.
    fn write(&self, bytes: &[u8]) -> Result<(), String> {
        let path = &self.path;
        let written = if self.force {
            store::write_atomic(path, bytes)
        } else {
            store::write_new(path, bytes)
        };
        written.map_err(|e| match e.kind() {
            io::ErrorKind::AlreadyExists => format!(
                "{} already exists; add --force to replace it",
                path.display()
            ),
            _ => format!("writing {}: {e}", path.display()),
        })
    }
}

fn snapshot_cmd(
    session: Option<String>,
    all: bool,
    out: Option<Out>,
    json: bool,
    theme: ThemeArg,
    stale_minutes: u64,
) -> Result<(), String> {
    let ext = out
        .as_ref()
        .and_then(|o| o.path.extension())
        .map(|e| e.to_string_lossy().to_ascii_lowercase());
    if let Some(o) = &out {
        if !matches!(ext.as_deref(), Some("png" | "svg" | "json")) {
            return Err(format!(
                "{}: --out needs a .png, .svg or .json file",
                o.path.display()
            ));
        }
    }
    let (graph, root) = load_graph(stale_minutes)?;

    if json || ext.as_deref() == Some("json") {
        let text = serde_json::to_string_pretty(&graph).expect("serializable") + "\n";
        match out {
            Some(out) => {
                out.write(text.as_bytes())?;
                println!("{}", out.path.display());
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

    let bytes = if ext.as_deref() == Some("svg") {
        svg.into_bytes()
    } else {
        crate::image::png(&svg)?
    };
    if let Some(out) = out {
        out.write(&bytes)?;
        println!("{}", out.path.display());
        return Ok(());
    }
    let path = {
        {
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
    if let Some(node) = running_in(graph) {
        return Ok(vec![node]);
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

/// The session this runs in, if it's recorded. Inside Claude Code, commands
/// can see it (`CLAUDE_CODE_SESSION_ID`).
pub(crate) fn running_in(graph: &Graph) -> Option<String> {
    let id = std::env::var("CLAUDE_CODE_SESSION_ID").ok()?;
    let node = format!("claude-code:{id}");
    graph.nodes.contains_key(&node).then_some(node)
}

/// `find_node`, among sessions only.
pub(crate) fn find_session(graph: &Graph, want: &str) -> Result<String, String> {
    let sessions = Graph {
        nodes: graph
            .nodes
            .iter()
            .filter(|(id, _)| !id.contains('/'))
            .map(|(id, n)| (id.clone(), n.clone()))
            .collect(),
        roots: Vec::new(),
    };
    find_node(&sessions, want)
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

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use clap::CommandFactory;
    use serde_json::Value;

    use super::*;

    fn text(value: &Value) -> &str {
        value.as_str().unwrap_or("").trim()
    }

    /// Every command, and every option of every command, has help text in
    /// docs/cli-help.json, and the file describes nothing that doesn't exist.
    #[test]
    fn every_command_and_option_has_help_text() {
        let json: Value =
            serde_json::from_str(help::JSON).expect("docs/cli-help.json is valid JSON");
        let documented: BTreeMap<&str, &Value> = json["commands"]
            .as_array()
            .expect("commands is a list")
            .iter()
            .map(|c| (text(&c["name"]), c))
            .collect();

        let cli = Cli::command();
        let mut commands = BTreeSet::new();
        for sub in cli.get_subcommands().filter(|s| !s.is_hide_set()) {
            let name = sub.get_name();
            commands.insert(name);
            let Some(doc) = documented.get(name) else {
                panic!("`agent-graph {name}` has no help text: add it to docs/cli-help.json");
            };
            assert!(!text(&doc["summary"]).is_empty(), "`{name}` has no summary");
            let description = doc["description"].as_array().map_or(0, Vec::len);
            assert!(description > 0, "`{name}` has no description");
            let examples = doc["examples"].as_array().cloned().unwrap_or_default();
            assert!(!examples.is_empty(), "`{name}` has no examples");
            for example in &examples {
                assert!(
                    !text(&example["command"]).is_empty() && !text(&example["text"]).is_empty(),
                    "`{name}` has an example without a command or an explanation"
                );
            }

            let options: BTreeMap<&str, &str> = doc["options"]
                .as_array()
                .map(|o| {
                    o.iter()
                        .map(|o| (text(&o["id"]), text(&o["text"])))
                        .collect()
                })
                .unwrap_or_default();
            let mut args = BTreeSet::new();
            for arg in sub.get_arguments() {
                let id = arg.get_id().as_str();
                if id == "help" || id == "version" {
                    continue;
                }
                args.insert(id);
                match options.get(id) {
                    None => panic!(
                        "`agent-graph {name}`'s `{id}` has no help text: add it to docs/cli-help.json"
                    ),
                    Some(&"") => panic!("`agent-graph {name}`'s `{id}` has empty help text"),
                    Some(_) => {}
                }
                assert!(
                    arg.get_help().is_some(),
                    "`{name}`'s `{id}` doesn't use its help text"
                );
            }
            for id in options.keys() {
                assert!(
                    args.contains(id),
                    "docs/cli-help.json describes `{id}` for `{name}`, which it doesn't have"
                );
            }
            assert!(
                sub.get_about().is_some() && sub.get_long_about().is_some(),
                "`{name}` isn't using its help text"
            );
        }
        for name in documented.keys() {
            assert!(
                commands.contains(name),
                "docs/cli-help.json describes `{name}`, which isn't a command"
            );
        }

        assert!(
            !text(&json["summary"]).is_empty(),
            "no summary for agent-graph itself"
        );
        assert!(
            json["sections"].as_array().is_some_and(|s| !s.is_empty()),
            "no overview sections"
        );
    }

    /// Each command's help is its full page, with examples.
    #[test]
    fn each_command_help_has_its_description_and_examples() {
        for sub in Cli::command()
            .get_subcommands()
            .filter(|s| !s.is_hide_set())
        {
            let long = sub
                .get_long_about()
                .map(|s| s.to_string())
                .unwrap_or_default();
            let after = sub
                .get_after_help()
                .map(|s| s.to_string())
                .unwrap_or_default();
            assert!(
                long.len() > 80,
                "`{}` has only a short description",
                sub.get_name()
            );
            assert!(
                after.starts_with("Examples:"),
                "`{}` has no examples",
                sub.get_name()
            );
        }
    }
}
