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
        #[arg(long, conflicts_with_all = ["command"], help = help::install::opt::CLOUD)]
        cloud: bool,
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
        #[arg(long, help = help::watch_remote::opt::SESSION)]
        session: Option<String>,
        #[arg(long, help = help::watch_remote::opt::NEW)]
        new: bool,
        #[arg(long, help = help::watch_remote::opt::LOGOUT)]
        logout: bool,
        #[arg(
            long,
            conflicts_with_all = ["session", "new", "logout", "no_autostart", "background"],
            help = help::watch_remote::opt::AUTOSTART
        )]
        autostart: bool,
        #[arg(
            long,
            conflicts_with_all = ["session", "new", "logout", "background"],
            help = help::watch_remote::opt::NO_AUTOSTART
        )]
        no_autostart: bool,
        #[arg(long, help = help::watch_remote::opt::BACKGROUND)]
        background: bool,
        #[arg(
            long,
            conflicts_with_all = ["session", "new", "logout", "autostart", "no_autostart", "background"],
            help = help::watch_remote::opt::CHECK
        )]
        check: bool,
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
            cloud,
        } => {
            // For Claude Code's cloud: the project's committed settings, and
            // no slash command (it'd run the CLI on the computer). For
            // Codex's: Codex's own files in the cloud's machine, from the
            // environment's setup script.
            let codex_cloud = cloud && matches!(provider, Provider::Codex);
            if cloud
                && (!matches!(provider, Provider::ClaudeCode | Provider::Codex)
                    || !matches!(scope, ScopeArg::User | ScopeArg::Project)
                    || (codex_cloud && !matches!(scope, ScopeArg::User)))
            {
                eprintln!(
                    "agent-graph: --cloud is for Claude Code, in a project's settings (--scope project), \
                     or for Codex, in its cloud environment's setup script"
                );
                return ExitCode::FAILURE;
            }
            if codex_cloud && std::env::var_os("CODEX_HOME").is_none_or(|v| v.is_empty()) {
                if !Path::new(install::CODEX_CLOUD_HOME).is_dir() {
                    eprintln!(
                        "agent-graph: install codex --cloud is for the setup script of a Codex cloud \
                         environment (chatgpt.com/codex), where Codex is in {}; it isn't here. \
                         On a computer, use `agent-graph install codex`.",
                        install::CODEX_CLOUD_HOME
                    );
                    return ExitCode::FAILURE;
                }
                // The setup script isn't told where Codex's home is, though
                // its tasks are. (Nothing else runs yet.)
                unsafe { std::env::set_var("CODEX_HOME", install::CODEX_CLOUD_HOME) };
            }
            install_cmd(
                provider.into(),
                if cloud && !codex_cloud {
                    Scope::Project
                } else {
                    scope.into()
                },
                InstallOptions {
                    dry_run,
                    yes,
                    hook_command: command,
                    slash_command: !no_slash_command && !cloud,
                    add: true,
                    cloud,
                },
            )
        }
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
                cloud: false,
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
            url, check: true, ..
        } => report_cloud_checks(&url, true),
        Command::WatchRemote {
            url,
            session,
            new,
            logout,
            autostart,
            no_autostart,
            background,
            check: _,
        } => paths::data_dir()
            .ok_or_else(|| "can't find your home directory".to_string())
            .and_then(|root| {
                if autostart {
                    return crate::autostart::enable(&root, &url);
                }
                if no_autostart {
                    return crate::autostart::disable();
                }
                crate::remote::run(
                    &root,
                    crate::remote::Options {
                        url,
                        session,
                        new,
                        logout,
                        background,
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
    /// The hooks for Claude Code's cloud (see `install::install_claude_code_cloud`).
    cloud: bool,
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
    } else if client == Client::Codex {
        match codex_hooks_changes(scope, &cwd, &opts)? {
            Some(codex) => changes.extend(codex),
            None if scope == Scope::Local => notes.push(
                "Codex has no project settings that aren't committed, so its hooks go in \
                 your own (--scope user) or the project's (--scope project)."
                    .into(),
            ),
            None => {}
        }
    } else if opts.add {
        notes.push(format!(
            "{} sessions aren't recorded yet: only Claude Code and Codex have hooks so far. \
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
            // There's nowhere uncommitted for a project's command, so
            // --scope local uses the project's folder too, where a
            // committed one is the project's (--scope project).
            let projects = (!opts.add && ours && scope == Scope::Local)
                .then(|| match committed(&cwd, &file.path) {
                    Ok(false) => None,
                    Ok(true) => Some(format!(
                        "Left the {} command ({}) alone: it's committed, so it's the \
                         project's. Remove it with --scope project.",
                        file.invoke,
                        file.path.display()
                    )),
                    Err(err) => Some(format!(
                        "Left the {} command ({}) alone: git couldn't say whether it's \
                         committed, as the project's would be. If it isn't, remove it \
                         with --scope project. What git said:\n{err}",
                        file.invoke,
                        file.path.display()
                    )),
                })
                .flatten();
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
                (false, Some(_)) if projects.is_some() => notes.extend(projects),
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
        // (Something was left alone, so it isn't "nothing installed".)
        if notes.iter().any(|n| n.starts_with("Left ")) {
            println!("Nothing else to do.");
            return Ok(());
        }
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
            if opts.cloud {
                let site = crate::remote::DEFAULT_URL;
                println!(
                    "Installed. Commit .claude/settings.json, then, in your Claude Code cloud \
                     environment's settings (claude.ai/code):\n\
                     \x20 1. Environment variables: AGENT_GRAPH_TOKEN=<an API token from {site}/account>\n\
                     \x20 2. Network access: Custom, allowing agentgraph.chofter.com and \
                     firebasestorage.googleapis.com\n\
                     Cloud sessions of this project are then shared live, at {site}/watch."
                );
                // Here, on a computer, the token's not needed: it's the
                // cloud environment's. In Claude's cloud (a setup script), it is.
                let in_cloud = std::env::var("CLAUDE_CODE_REMOTE").is_ok_and(|v| v == "true");
                report_cloud_checks(site, in_cloud)?;
            } else {
                println!("Installed. New Claude Code sessions will be recorded in {data}.");
                offer_autostart(opts.yes);
            }
        } else if client == Client::Codex && opts.cloud {
            let site = crate::remote::DEFAULT_URL;
            println!(
                "Installed. Codex's cloud tasks in this environment will be recorded, and shared \
                 live at {site}/watch once the environment's settings (chatgpt.com/codex) have:\n\
                 \x20 1. Environment variables: AGENT_GRAPH_TOKEN=<an API token from {site}/account>. \
                 Make it an environment variable, not a secret: a secret reaches only the setup \
                 script, and it's the tasks that share.\n\
                 \x20 2. Agent internet access: On, with agentgraph.chofter.com in Additional \
                 allowed domains, and All methods allowed"
            );
            report_cloud_checks(site, true)?;
            save_cloud_login(site)?;
        } else if client == Client::Codex && scope != Scope::Local {
            let data = paths::data_dir()
                .map(|d| d.display().to_string())
                .unwrap_or_default();
            println!("Installed. New Codex sessions will be recorded in {data}.");
            if scope == Scope::Project {
                println!(
                    "Codex only runs a project's hooks once the project is trusted in Codex \
                     (it asks, the first time you open it)."
                );
            }
            offer_autostart(opts.yes);
            // Last, so nothing scrolls it out of sight.
            println!("\n{}", restart_notice(true, chatgpt_app(), color_out()));
        } else {
            println!("Installed.");
        }
    } else {
        println!("Uninstalled.");
        if client == Client::Codex && scope != Scope::Local {
            println!("\n{}", restart_notice(false, chatgpt_app(), color_out()));
        }
    }
    Ok(())
}

/// Checks, and prints, what sharing from a cloud needs (see
/// `remote::cloud_checks`): reaching `url`, and a valid `AGENT_GRAPH_TOKEN`.
/// An error if they aren't in place, so a setup script stops there. Without
/// `token_needed` (installing on a computer, for a cloud whose settings
/// hold the token), a token that isn't set here is only noted; one that is
/// set must still be valid.
fn report_cloud_checks(url: &str, token_needed: bool) -> Result<(), String> {
    println!("\nChecking what sharing from the cloud needs:");
    let token_set = std::env::var(crate::account::TOKEN_VAR).is_ok_and(|t| !t.trim().is_empty());
    let mut failed = false;
    for (i, check) in crate::remote::cloud_checks(url).into_iter().enumerate() {
        // (The second check is the token's.)
        if i == 1 && !token_needed && !token_set {
            println!(
                "• {} isn't set here, which is fine: it goes in the cloud environment's settings.",
                crate::account::TOKEN_VAR
            );
            continue;
        }
        failed |= !check.ok;
        println!("  {}", check.line);
    }
    if failed {
        Err(
            "not everything sharing needs is in place (see above), so nothing will be shared yet"
                .into(),
        )
    } else {
        println!("Everything sharing needs is in place.");
        Ok(())
    }
}

/// Saves `AGENT_GRAPH_TOKEN` as this environment's login (checked: see
/// `account::from_env`), installing for Codex's cloud. Codex gives the
/// environment's variables to the commands its agent runs, but not to the
/// hooks it runs itself: the sharing its `SessionStart` hook starts would
/// find no token. The setup script, where this runs, has it, and what it
/// leaves on disk is there in the tasks.
fn save_cloud_login(site: &str) -> Result<(), String> {
    let root = paths::data_dir().ok_or("can't find your home directory")?;
    let client = crate::remote::Client::new(site);
    match crate::account::from_env(&root, site, &client)
        .map_err(crate::account::TokenError::message)?
    {
        Some(account) => {
            println!(
                "  ✓ {} saved as this environment's login{}, for the sharing Codex's hooks start \
                 (Codex doesn't give its hooks the environment's variables).",
                crate::account::TOKEN_VAR,
                account.email.map(|e| format!(" ({e})")).unwrap_or_default()
            );
            Ok(())
        }
        None => Err(format!("{} isn't set", crate::account::TOKEN_VAR)),
    }
}

/// Whether the ChatGPT desktop app is installed (it runs Codex too).
fn chatgpt_app() -> bool {
    cfg!(target_os = "macos") && paths::chatgpt_apps().iter().any(|a| a.is_dir())
}

/// Whether what's printed can be in colour: a terminal, without `NO_COLOR`.
fn color_out() -> bool {
    io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty())
}

/// What to do after changing Codex's hooks, in a box (bold yellow, with
/// `color`), so it isn't missed: Codex reads its hooks only when it starts,
/// so the ChatGPT app (if `chatgpt`) and any Codex already running in a
/// terminal have to be restarted for the change to take.
fn restart_notice(installed: bool, chatgpt: bool, color: bool) -> String {
    let mut lines = vec![if chatgpt {
        "RESTART THE CHATGPT APP NOW".to_string()
    } else {
        "RESTART CODEX NOW".to_string()
    }];
    lines.push(String::new());
    lines.push("Codex only reads its hooks when it starts, so until then".into());
    lines.push(if installed {
        "its sessions aren't recorded.".into()
    } else {
        "it keeps running Agent Graph's.".into()
    });
    lines.push(String::new());
    if chatgpt {
        lines.push("- ChatGPT: quit it (Cmd+Q, not just closing its window),".into());
        lines.push("  then open it again.".into());
    }
    lines.push("- Codex in a terminal: exit any that's running, and start".into());
    lines.push("  it again (`codex resume` picks up where you were).".into());
    let width = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
    let (on, off) = if color {
        ("\x1b[1;33m", "\x1b[0m")
    } else {
        ("", "")
    };
    let rule = "━".repeat(width + 4);
    let mut out = format!("{on}┏{rule}┓{off}\n");
    for line in &lines {
        let pad = " ".repeat(width - line.chars().count());
        out.push_str(&format!("{on}┃  {line}{pad}  ┃{off}\n"));
    }
    out.push_str(&format!("{on}┗{rule}┛{off}"));
    out
}

/// After installing: offers to share live whenever this computer's logged
/// in to (`watch-remote --autostart`), asking first, yes by default. Not
/// logged in, and without an API token, it first offers to log in (in the
/// browser), or says how to sign up and make a token. With --yes, or
/// without a terminal to ask in, it only says how. Nothing if it's set up
/// already, or can't be here.
fn offer_autostart(yes: bool) {
    let site = crate::remote::DEFAULT_URL;
    if crate::autostart::service_path().is_err() || crate::autostart::enabled() {
        return;
    }
    let Some(root) = paths::data_dir() else {
        return;
    };
    let mut account = crate::account::load(&root, site);
    let token = std::env::var(crate::account::TOKEN_VAR).is_ok_and(|t| !t.trim().is_empty());
    if yes || !io::stdin().is_terminal() {
        println!(
            "\nTo see your sessions live from anywhere (your phone, say), shared to your account at \
             {site}/watch whenever you log in to this computer: agent-graph watch-remote --autostart"
        );
        if account.is_none() && !token {
            println!("{}", sign_up_note(site));
        }
        return;
    }
    // Sharing needs an account: logged in first, if you'd like to be.
    if account.is_none() && !token {
        let question = format!(
            "\nYour sessions can also be shared live to your account at {site}/watch, to see them \
             from anywhere (your phone, say). Log in to {site} now? (It opens your browser.)"
        );
        match ask(&question, true) {
            Ok(true) => {
                let client = crate::remote::Client::new(site);
                match crate::account::login(&root, site, &client) {
                    Ok(logged_in) => account = Some(logged_in),
                    Err(e) => {
                        println!("Couldn't log in: {e}\n{}", sign_up_note(site));
                        return;
                    }
                }
            }
            Ok(false) => {
                println!("{}", sign_up_note(site));
                return;
            }
            Err(_) => return,
        }
    }
    let question = autostart_question(site, account.as_ref(), token);
    // Yes, unless you say otherwise: it's what most people want.
    match ask(&question, true) {
        Ok(true) => {
            if let Err(e) = crate::autostart::enable(&root, site) {
                println!("Couldn't set that up: {e}");
            }
        }
        Ok(false) => println!("Not now. To later: agent-graph watch-remote --autostart"),
        Err(_) => {}
    }
}

/// How to share live without logging in now: sign up, and either log in
/// later or give an API token from the account page.
fn sign_up_note(site: &str) -> String {
    format!(
        "To share them later: sign up at {site} if you haven't, then run `agent-graph \
         watch-remote --autostart`, which logs you in, in your browser. Or make an API token on \
         your account page ({site}/account), set {}=<the token>, and run it then.",
        crate::account::TOKEN_VAR
    )
}

/// The question `offer_autostart` asks, saying which login sharing will use:
/// an API token in `AGENT_GRAPH_TOKEN` (`token`), which setting it up saves
/// as the login (see `account::from_env`), else the one already saved, else
/// one made in the browser first.
fn autostart_question(
    site: &str,
    account: Option<&crate::account::Account>,
    token: bool,
) -> String {
    let login = match (token, account) {
        (true, _) => format!(
            " (It'll use your API token, from {}.)",
            crate::account::TOKEN_VAR
        ),
        (false, Some(account)) => match &account.email {
            Some(email) => format!(" (As {email}, logged in already.)"),
            None => String::new(),
        },
        (false, None) => " (You'll log in to the site first, in your browser.)".to_string(),
    };
    format!(
        "\nAlso share your sessions live to your account at {site}/watch, to see them from anywhere, \
         whenever you log in to this computer?{login}"
    )
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
    if opts.add && opts.cloud {
        install::install_claude_code_cloud(&mut after, crate::remote::DEFAULT_URL)?;
    } else if opts.add {
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
    if opts.add && opts.cloud {
        summary.push(format!(
            "Adds hooks for Claude Code's cloud (claude.ai/code): {}",
            install::our_events(&after).join(", ")
        ));
        summary.push(
            "They do nothing on a computer. In a cloud session, they install agent-graph if it \
             isn't there, record the session, and share it live."
                .into(),
        );
        if !install::our_events(&before).is_empty() {
            summary.push("(Replaces the Agent Graph hooks already there.)".into());
        }
    } else if opts.add {
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
                     until it is."
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
    let backup = link.with_extension("json.agent-graph.bak");
    // Settings that were only ever our hooks (installing made the file, so
    // there's no backup of one) go, rather than staying behind empty.
    let remove = !opts.add
        && !is_link
        && after == serde_json::json!({})
        && std::fs::symlink_metadata(&backup).is_err();
    if remove {
        summary.push("Removes the settings file: nothing else is in it.".into());
    }
    Ok(Some(Change {
        path,
        contents: (!remove)
            .then(|| serde_json::to_string_pretty(&after).expect("serializable") + "\n"),
        summary,
        // Only a file without our hooks is backed up: the settings as they
        // were without Agent Graph, which installing again (or
        // uninstalling) mustn't replace with a copy that has them.
        backup: (existed && install::our_events(&before).is_empty()).then_some(backup),
    }))
}

/// The changes to Codex's hooks file, and to its `config.toml` to trust
/// them (see `install::codex_trust`), if any. None for `--scope local`:
/// Codex has nowhere for it.
fn codex_hooks_changes(
    scope: Scope,
    cwd: &Path,
    opts: &InstallOptions,
) -> Result<Option<Vec<Change>>, String> {
    let Some(link) = install::codex_hooks_path(scope, cwd) else {
        if scope == Scope::Local {
            return Ok(None);
        }
        return Err("can't find your home directory".into());
    };
    inside_project(scope, cwd, &link)?;
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
    let command = match &opts.hook_command {
        Some(c) => c.clone(),
        None if scope == Scope::Project => install::path_command("codex"),
        None => install::default_command("codex")?,
    };
    let mut after = before.clone();
    if opts.add && opts.cloud {
        install::install_codex_cloud(&mut after, &command)?;
    } else if opts.add {
        install::install_codex(&mut after, &command)?;
    } else {
        install::uninstall_codex(&mut after)?;
    }

    // Codex trusts each hook by where it is in the file, by the file's full
    // path as Codex sees it (with links followed).
    let source = as_codex_sees(&link).display().to_string();
    let config_path = install::codex_config_path().ok_or("can't find your home directory")?;
    let config_existed = config_path.exists();
    let config_before = if config_existed {
        std::fs::read_to_string(&config_path)
            .map_err(|e| format!("reading {}: {e}", config_path.display()))?
    } else {
        String::new()
    };
    let trusted = install::codex_trust(&config_before, &source, &before, &after)?;
    // Plans are only recorded with Codex's plan tool on. (Only for your
    // own hooks: a project's are shared, and this is your own setting.)
    let plans = scope == Scope::User;
    let config_after = if plans {
        install::codex_plan_tool(&trusted, opts.add)?
    } else {
        trusted.clone()
    };

    let mut changes = Vec::new();
    if after != before {
        let mut summary = vec![if is_link {
            format!(
                "Codex hooks file: {} (a link to {})",
                link.display(),
                path.display()
            )
        } else {
            format!("Codex hooks file: {}", path.display())
        }];
        if opts.add {
            summary.push(format!("Hook command:  {command}"));
            summary.push(format!(
                "Adds hooks for: {}",
                install::our_codex_events(&after).join(", ")
            ));
            if !install::our_codex_events(&before).is_empty() {
                summary.push("(Replaces the Agent Graph hooks already there.)".into());
            }
            if scope == Scope::Project && opts.hook_command.is_none() {
                summary.push(
                    "Project settings are shared, so the hooks run `agent-graph` from PATH: \
                     everyone who uses the project needs it installed."
                        .into(),
                );
            }
        } else {
            summary.push(format!(
                "Removes hooks for: {}",
                install::our_codex_events(&before).join(", ")
            ));
        }
        let backup = link.with_extension("json.agent-graph.bak");
        let remove = !opts.add
            && !is_link
            && after == serde_json::json!({})
            && std::fs::symlink_metadata(&backup).is_err();
        if remove {
            summary.push("Removes the hooks file: nothing else is in it.".into());
        }
        changes.push(Change {
            path,
            contents: (!remove)
                .then(|| serde_json::to_string_pretty(&after).expect("serializable") + "\n"),
            summary,
            backup: (existed && install::our_codex_events(&before).is_empty()).then_some(backup),
        });
    }
    if config_after != config_before {
        let backup = config_path.with_extension("toml.agent-graph.bak");
        let mut summary = vec![format!("Codex settings: {}", config_path.display())];
        if trusted != config_before {
            summary.push(if opts.add {
                "Trusts Agent Graph's hooks, as Codex's /hooks would, so Codex runs them \
                 (it won't run a hook until it's trusted)."
                    .into()
            } else {
                "Removes the trust Agent Graph's hooks had.".into()
            });
        }
        if config_after != trusted {
            summary.push(if opts.add {
                "Turns on Codex's plan tool ([tools.update_plan]), off by default, so a \
                 session's plan shows as its tasks."
                    .into()
            } else {
                "Turns Codex's plan tool back off, as Agent Graph turned it on.".into()
            });
        }
        summary.push("Nothing else is changed.".into());
        changes.push(Change {
            summary,
            backup: (config_existed && std::fs::symlink_metadata(&backup).is_err())
                .then_some(backup),
            path: config_path,
            contents: Some(config_after),
        });
    }
    Ok(Some(changes).filter(|c: &Vec<Change>| !c.is_empty()))
}

/// `path` as Codex names it in its trust keys: absolute, with links
/// followed as far as the path exists, and on Windows without the `\\?\`
/// prefix canonical paths get, as Codex has it (it uses `dunce` too).
fn as_codex_sees(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|d| d.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };
    let mut existing = absolute.as_path();
    let mut rest = Vec::new();
    loop {
        if let Ok(real) = dunce::canonicalize(existing) {
            return rest.iter().rev().fold(real, |p, part| p.join(part));
        }
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_os_string());
                existing = parent;
            }
            _ => return absolute,
        }
    }
}

/// Whether `path` (in `dir`) is committed in the git repository `dir` is
/// in: in its last commit, not just staged. Without git, outside a
/// repository, or before its first commit, it isn't. An error says git
/// couldn't tell (it refuses a repository someone else owns, say).
fn committed(dir: &Path, path: &Path) -> Result<bool, String> {
    let Ok(relative) = path.strip_prefix(dir) else {
        return Ok(false);
    };
    let mut git = std::process::Command::new("git");
    // The repository `dir` is in, not one the environment names (a git
    // hook's, say).
    for var in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_COMMON_DIR",
    ] {
        git.env_remove(var);
    }
    let Ok(out) = git
        .arg("-C")
        .arg(dir)
        .args(["ls-tree", "HEAD", "--"])
        .arg(relative)
        .env("LC_ALL", "C")
        .stdin(std::process::Stdio::null())
        .output()
    else {
        return Ok(false);
    };
    if out.status.success() {
        return Ok(!out.stdout.is_empty());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    if err.contains("not a git repository") || err.contains("Not a valid object name HEAD") {
        return Ok(false);
    }
    Err(err.trim().to_string())
}

fn apply(change: &Change) -> Result<(), String> {
    let path = &change.path;
    let Some(text) = &change.contents else {
        std::fs::remove_file(path).map_err(|e| format!("removing {}: {e}", path.display()))?;
        // Don't leave empty folders behind: a skill's own folder, then the
        // skills/commands folder, then the agent's (.claude, .agents...),
        // each if nothing else is in it (remove_dir only removes empty
        // folders).
        let named = |d: &Path, names: &[&str]| {
            d.file_name()
                .is_some_and(|n| names.iter().any(|name| n == *name))
        };
        let mut dir = path.parent();
        if let Some(d) = dir.filter(|d| slash::is_own_folder(d)) {
            let _ = std::fs::remove_dir(d);
            dir = d.parent();
        }
        if let Some(d) = dir.filter(|d| named(d, &["skills", "commands"])) {
            let _ = std::fs::remove_dir(d);
            dir = d.parent();
        }
        if let Some(d) =
            dir.filter(|d| named(d, &[".claude", ".agents", ".codex", ".gemini", ".cursor"]))
        {
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
    ask(question, false)
}

/// Asks a yes-or-no `question` in the terminal; Enter alone answers
/// `default`. (The input ending, with Ctrl+D, is a no, whatever the default.)
fn ask(question: &str, default: bool) -> Result<bool, String> {
    if !io::stdin().is_terminal() {
        return Err(
            "not running in a terminal; re-run with --yes to confirm, or --dry-run to preview"
                .into(),
        );
    }
    print!("{question} {} ", if default { "[Y/n]" } else { "[y/N]" });
    io::stdout().flush().ok();
    let mut answer = String::new();
    let read = io::stdin()
        .lock()
        .read_line(&mut answer)
        .map_err(|e| e.to_string())?;
    Ok(read > 0 && answer_is_yes(&answer, default))
}

/// Whether `answer` (a line typed at a yes-or-no question) means yes:
/// anything starting with y; nothing at all means `default`.
fn answer_is_yes(answer: &str, default: bool) -> bool {
    match answer.trim().to_ascii_lowercase().chars().next() {
        None => default,
        Some(c) => c == 'y',
    }
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
    for file in &loaded.unreadable {
        eprintln!("agent-graph: couldn't read {}; left it out", file.display());
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
            "Run `agent-graph install claude-code` (or `install codex`), then start a new session."
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
    let (mut graph, root) = load_graph(stale_minutes)?;
    let none = || {
        format!(
            "no sessions recorded yet in {}. Run `agent-graph install claude-code` (or `install codex`), then start a new session.",
            root.display()
        )
    };

    if json || ext.as_deref() == Some("json") {
        // The sessions asked for, as a picture would have; with neither
        // option, the whole graph (which may be empty).
        if session.is_some() || all {
            if graph.roots.is_empty() {
                return Err(none());
            }
            let cwd = std::env::current_dir().ok();
            let roots = pick_roots(&graph, session.as_deref(), all, cwd.as_deref())?;
            graph = only(graph, &roots);
        }
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
        return Err(none());
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
    let dir = root.join("images");
    store::ensure_dir(&dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    let stamp = humantime::format_rfc3339_seconds(SystemTime::now())
        .to_string()
        .replace(['-', ':'], "")
        .replace('T', "-")
        .trim_end_matches('Z')
        .to_string();
    // The name only has seconds, so another snapshot may have it already:
    // that one is kept, and this one gets the next free "-2", "-3"...
    let mut n = 1;
    let path = loop {
        let name = match n {
            1 => format!("agent-graph-{stamp}.png"),
            _ => format!("agent-graph-{stamp}-{n}.png"),
        };
        let path = dir.join(name);
        match store::write_new(&path, &bytes) {
            Ok(()) => break path,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists && n < 1000 => n += 1,
            Err(e) => return Err(format!("writing {}: {e}", path.display())),
        }
    };
    // Only the path goes to stdout, so scripts and agents can use it directly.
    println!("{}", path.display());
    Ok(())
}

/// `graph` cut down to the sessions `picked` (sessions or agents) are in,
/// in that order, then those they wait on, directly or not (which what's
/// blocked counts), and those they exchanged messages with (but not who
/// those did in turn: a lead's team, say): whole trees, each from a root.
pub fn only(mut graph: Graph, picked: &[String]) -> Graph {
    let top = |id: &str| {
        let mut id = id;
        let mut seen = std::collections::BTreeSet::new();
        while let Some(parent) = graph.nodes[id].parent.as_deref() {
            if !graph.nodes.contains_key(parent) || !seen.insert(id) {
                break;
            }
            id = parent;
        }
        id.to_string()
    };
    let mut roots: Vec<String> = Vec::new();
    let mut keep = std::collections::BTreeSet::new();
    // Each tree, and whether it was picked.
    let mut queue: std::collections::VecDeque<(String, bool)> = picked
        .iter()
        .filter(|id| graph.nodes.contains_key(*id))
        .map(|id| (top(id), true))
        .collect();
    while let Some((root, was_picked)) = queue.pop_front() {
        if roots.contains(&root) {
            continue;
        }
        roots.push(root.clone());
        let mut peers = Vec::new();
        let mut named = Vec::new();
        let mut next = vec![root];
        while let Some(id) = next.pop() {
            let Some(node) = graph.nodes.get(&id) else {
                continue;
            };
            if !keep.insert(id) {
                continue;
            }
            next.extend(node.children.iter().cloned());
            named.extend(node.spawns.iter().filter_map(|s| s.child.as_ref()));
            named.extend(node.waits.iter().filter_map(|w| w.on.as_ref()));
            named.extend(node.blocked.iter().flat_map(|b| &b.on));
            if was_picked {
                peers.extend(node.messages.iter().map(|m| &m.peer));
            }
        }
        let tops = |ids: Vec<&String>, picked: bool| {
            ids.into_iter()
                .filter(|id| graph.nodes.contains_key(*id))
                .map(|id| (top(id), picked))
                .collect::<Vec<_>>()
        };
        queue.extend(tops(named, false));
        queue.extend(tops(peers, false));
    }
    graph.nodes.retain(|id, _| keep.contains(id));
    graph.late.retain(|id| keep.contains(id));
    graph.roots = roots;
    graph
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

/// The session this runs in, if it's recorded: the one whose shell this is,
/// as a session started here would find its parent (Codex's shell names its
/// thread; Agent Graph's hooks give Claude Code's its session), or else, in
/// Claude Code, `CLAUDE_CODE_SESSION_ID`.
pub(crate) fn running_in(graph: &Graph) -> Option<String> {
    let shell =
        crate::link::parent_here("").and_then(|node| node.split('/').next().map(str::to_string));
    let claude = std::env::var("CLAUDE_CODE_SESSION_ID")
        .ok()
        .map(|id| format!("claude-code:{id}"));
    [shell, claude]
        .into_iter()
        .flatten()
        .find(|node| graph.nodes.contains_key(node))
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
        late: Default::default(),
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

    #[test]
    fn enter_alone_takes_the_default() {
        for (answer, default, yes) in [
            ("\n", true, true),
            ("\n", false, false),
            ("  \n", true, true),
            ("y\n", false, true),
            ("Yes\n", false, true),
            ("yea\n", false, true),
            ("n\n", true, false),
            ("No\n", true, false),
            ("maybe\n", true, false),
        ] {
            assert_eq!(
                answer_is_yes(answer, default),
                yes,
                "{answer:?} with default {default}"
            );
        }
    }

    #[test]
    fn the_autostart_question_says_which_login_it_uses() {
        let site = "https://agentgraph.chofter.com";
        let account = crate::account::Account {
            site: site.into(),
            token: "t".into(),
            email: Some("me@example.com".into()),
        };
        let token = autostart_question(site, None, true);
        assert!(
            token.contains("API token, from AGENT_GRAPH_TOKEN"),
            "{token}"
        );
        assert!(!token.contains("browser"), "{token}");
        // A token wins over a saved login: setting up saves it as the login.
        assert!(autostart_question(site, Some(&account), true).contains("API token"));
        assert!(autostart_question(site, Some(&account), false).contains("As me@example.com"));
        assert!(autostart_question(site, None, false).contains("in your browser"));
        let note = sign_up_note(site);
        assert!(
            note.contains("sign up at https://agentgraph.chofter.com"),
            "{note}"
        );
        assert!(
            note.contains("/account") && note.contains("AGENT_GRAPH_TOKEN"),
            "{note}"
        );
    }

    #[test]
    fn the_restart_notice_is_boxed_and_says_what_to_restart() {
        let plain = restart_notice(true, true, false);
        assert!(plain.starts_with("┏━"), "{plain}");
        assert!(plain.contains("RESTART THE CHATGPT APP NOW"), "{plain}");
        assert!(plain.contains("Cmd+Q"), "{plain}");
        assert!(!plain.contains('\x1b'), "no colour unless asked: {plain}");
        // Every line of the box is as wide as the rest.
        let widths: Vec<usize> = plain.lines().map(|l| l.chars().count()).collect();
        assert!(widths.iter().all(|w| *w == widths[0]), "{plain}");
        let terminal_only = restart_notice(false, false, false);
        assert!(terminal_only.contains("RESTART CODEX NOW"));
        assert!(!terminal_only.contains("ChatGPT"));
        assert!(terminal_only.contains("keeps running Agent Graph's"));
        assert!(restart_notice(true, true, true).starts_with("\x1b[1;33m┏"));
    }

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
