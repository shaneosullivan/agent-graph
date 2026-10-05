//! `agent-graph diagnostics`: what's in place for recording and sharing,
//! here (a computer of one's own, or a coding agent's cloud), and what
//! isn't, with how to fix it.
//!
//! Each problem is one of `docs/troubleshooting.json`'s (compiled in), by
//! its id: the fix said here is the one the site's /troubleshooting page
//! has, at `#<id>`. Nothing secret is reported: a token is only said to be
//! set or not, and any that turns up in a log is hidden.
//!
//! In a cloud, `watch-remote` sends a report to the site when it starts and
//! when it stops (`report_from_cloud`), so a cloud that's broken says so at
//! /watch, where nothing else would.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::install::{self, Scope};
use crate::remote::{Client, Ping, SendError};

/// What can go wrong, and how to fix it: docs/troubleshooting.json.
pub const CATALOGUE: &str = include_str!("../docs/troubleshooting.json");

#[derive(Deserialize)]
struct Catalogue {
    issues: Vec<Issue>,
}

/// One kind of problem, as the catalogue has it.
#[derive(Deserialize, Clone)]
pub struct Issue {
    pub id: String,
    pub title: String,
    pub problem: String,
    pub fix: String,
    /// The fix in each cloud, where it's different, by the cloud's name.
    #[serde(default)]
    pub clouds: BTreeMap<String, String>,
}

impl Issue {
    /// The fix, in `cloud` (or on a computer of one's own, for `None`).
    pub fn fix_in(&self, cloud: Option<&str>) -> &str {
        cloud.and_then(|c| self.clouds.get(c)).unwrap_or(&self.fix)
    }
}

/// The catalogue's issue `id`.
pub fn issue(id: &str) -> Option<&'static Issue> {
    static ISSUES: OnceLock<Vec<Issue>> = OnceLock::new();
    ISSUES
        .get_or_init(|| {
            serde_json::from_str::<Catalogue>(CATALOGUE)
                .expect("docs/troubleshooting.json is valid")
                .issues
        })
        .iter()
        .find(|i| i.id == id)
}

/// Where the site explains issue `id`, and how to fix it.
pub fn link(site: &str, id: &str) -> String {
    format!("{}/troubleshooting#{id}", site.trim_end_matches('/'))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    /// In place.
    Ok,
    /// Worth knowing, and nothing wrong.
    Info,
    /// Something that may stop recording or sharing, or will one day.
    Warn,
    /// Something that stops recording or sharing.
    Error,
}

/// One thing checked.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub level: Level,
    /// What was found, in a line.
    pub what: String,
    /// The catalogue's issue, for anything that isn't in place.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issue: Option<String>,
    /// More about it: the file, the error.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl Finding {
    fn ok(what: impl Into<String>) -> Finding {
        Finding {
            level: Level::Ok,
            what: what.into(),
            issue: None,
            detail: None,
        }
    }

    fn info(what: impl Into<String>) -> Finding {
        Finding {
            level: Level::Info,
            ..Finding::ok(what)
        }
    }

    fn problem(level: Level, issue: &str, what: impl Into<String>) -> Finding {
        debug_assert!(
            self::issue(issue).is_some(),
            "{issue} isn't in the catalogue"
        );
        Finding {
            level,
            what: what.into(),
            issue: Some(issue.to_string()),
            detail: None,
        }
    }

    fn with_detail(mut self, detail: impl Into<String>) -> Finding {
        self.detail = Some(detail.into());
        self
    }
}

/// Everything checked, and where.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    /// This agent-graph's version.
    pub version: String,
    /// The coding agent's cloud this is, or none on a computer of one's own.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cloud: Option<String>,
    /// The operating system and processor: "linux x86_64".
    pub platform: String,
    pub site: String,
    /// When (RFC 3339).
    pub at: String,
    pub findings: Vec<Finding>,
    /// The ends of agent-graph's own logs, where there are any (in a cloud,
    /// what sharing and installing said), with tokens hidden.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub logs: BTreeMap<String, String>,
}

impl Report {
    /// The issues that stop recording or sharing, each once, in order: what
    /// makes one cloud's breakage the same as another's.
    pub fn errors(&self) -> Vec<&str> {
        let mut ids: Vec<&str> = self
            .findings
            .iter()
            .filter(|f| f.level == Level::Error)
            .filter_map(|f| f.issue.as_deref())
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    fn count(&self, level: Level) -> usize {
        self.findings.iter().filter(|f| f.level == level).count()
    }

    /// The report as text, for a terminal.
    pub fn text(&self) -> String {
        let mut out = format!(
            "Agent Graph diagnostics: agent-graph {}, {} ({})\n\n",
            self.version,
            self.cloud
                .as_deref()
                .map_or_else(|| "on this computer".to_string(), |c| format!("in {c}")),
            self.platform
        );
        for f in &self.findings {
            let mark = match f.level {
                Level::Ok => "✓",
                Level::Info => "•",
                Level::Warn => "!",
                Level::Error => "✗",
            };
            out.push_str(&format!("  {mark} {}\n", f.what));
            if let Some(detail) = &f.detail {
                for line in detail.lines() {
                    out.push_str(&format!("      {line}\n"));
                }
            }
            if let Some(issue) = f.issue.as_deref().and_then(issue) {
                if f.level >= Level::Warn {
                    out.push_str(&format!(
                        "      Fix: {}\n",
                        issue.fix_in(self.cloud.as_deref())
                    ));
                }
                out.push_str(&format!("      More: {}\n", link(&self.site, &issue.id)));
            }
        }
        for (name, tail) in &self.logs {
            out.push_str(&format!("\nThe end of {name}:\n"));
            for line in tail.lines() {
                out.push_str(&format!("  | {line}\n"));
            }
        }
        let (errors, warnings) = (self.count(Level::Error), self.count(Level::Warn));
        out.push('\n');
        out.push_str(&match (errors, warnings) {
            (0, 0) => "Everything Agent Graph needs is in place.".to_string(),
            (0, w) => format!("No problems, but {w} warning{}.", plural(w)),
            (e, 0) => format!("{e} problem{}.", plural(e)),
            (e, w) => format!("{e} problem{} and {w} warning{}.", plural(e), plural(w)),
        });
        out.push('\n');
        out
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// What to check.
pub struct Options {
    /// The site sharing goes to.
    pub site: String,
    /// The folder it's run in: a project whose hooks may be committed.
    pub project: PathBuf,
    /// Run by `watch-remote` itself, which is the sharing: whether another's
    /// sharing isn't checked.
    pub from_sharer: bool,
}

/// Checks everything, here.
pub fn run(opts: &Options) -> Report {
    let cloud = crate::account::cloud();
    let client = Client::new(&opts.site);
    let mut findings = Vec::new();
    let root = crate::paths::data_dir();

    data_folder(root.as_deref(), cloud, opts.from_sharer, &mut findings);
    if cloud.is_none() {
        local_hooks(&opts.project, &mut findings);
    }
    project_hooks(&opts.project, cloud, &mut findings);
    if cloud.is_some() {
        checkout_current(&opts.project, &mut findings);
    }
    if cloud == Some(crate::account::CODEX_CLOUD) {
        codex_cloud_hooks(&mut findings);
    }
    let reached = site(&client, &opts.site, cloud, &mut findings);
    let logged_in = login(
        &client,
        root.as_deref(),
        &opts.site,
        cloud,
        reached,
        &mut findings,
    );
    if !opts.from_sharer && logged_in {
        sharing(root.as_deref(), &opts.site, cloud, &mut findings);
    }
    if reached {
        version(&client, &mut findings);
    }
    let logs = root
        .as_deref()
        .map(|root| logs(root, cloud, &mut findings))
        .unwrap_or_default();

    Report {
        version: env!("CARGO_PKG_VERSION").to_string(),
        cloud: cloud.map(String::from),
        platform: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        site: opts.site.trim_end_matches('/').to_string(),
        at: humantime::format_rfc3339_seconds(crate::clock::now()).to_string(),
        findings,
        logs,
    }
}

/// The data folder: there, written to, and what's in it.
/// (Sharing starts as the session does, before its first event's recorded:
/// then, none yet is only said.)
fn data_folder(
    root: Option<&Path>,
    cloud: Option<&str>,
    from_sharer: bool,
    out: &mut Vec<Finding>,
) {
    let Some(root) = root else {
        out.push(Finding::problem(
            Level::Error,
            "data-folder",
            "There's no home folder to keep sessions in",
        ));
        return;
    };
    let probe = root.join(".diagnostics");
    if let Err(e) = std::fs::create_dir_all(root).and_then(|()| std::fs::write(&probe, b"ok")) {
        out.push(
            Finding::problem(
                Level::Error,
                "data-folder",
                format!("Sessions can't be recorded in {}", root.display()),
            )
            .with_detail(e.to_string()),
        );
        return;
    }
    let _ = std::fs::remove_file(&probe);
    let events = crate::paths::events_dir(root);
    let files: Vec<(SystemTime, String)> = std::fs::read_dir(&events)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
        .filter_map(|e| {
            let modified = e.metadata().ok()?.modified().ok()?;
            Some((modified, e.file_name().to_string_lossy().into_owned()))
        })
        .collect();
    match files.iter().max() {
        None if from_sharer => out.push(Finding::info(format!(
            "No sessions recorded in {} yet",
            root.display()
        ))),
        None => out.push(Finding::problem(
            if cloud.is_some() {
                Level::Error
            } else {
                Level::Warn
            },
            "no-sessions",
            format!("No sessions have been recorded in {}", root.display()),
        )),
        Some((last, _)) => out.push(Finding::ok(format!(
            "{} session file{} recorded in {}, the last {}",
            files.len(),
            plural(files.len()),
            root.display(),
            ago(*last)
        ))),
    }
}

fn ago(at: SystemTime) -> String {
    let secs = at.elapsed().unwrap_or(Duration::ZERO).as_secs();
    match secs {
        0..60 => "just now".to_string(),
        60..3600 => format!("{}m ago", secs / 60),
        3600..86400 => format!("{}h ago", secs / 3600),
        _ => format!("{}d ago", secs / 86400),
    }
}

/// Every `command` in a hooks or settings file, wherever it is.
fn commands(value: &Value) -> Vec<&str> {
    match value {
        Value::Object(map) => map
            .iter()
            .flat_map(|(k, v)| match (k.as_str(), v) {
                ("command", Value::String(c)) => vec![c.as_str()],
                _ => commands(v),
            })
            .collect(),
        Value::Array(items) => items.iter().flat_map(commands).collect(),
        _ => Vec::new(),
    }
}

/// A hooks file at `path`, as JSON: `None` if there's none, and an error
/// finding if it can't be read.
fn read_json(path: &Path, label: &str, out: &mut Vec<Finding>) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str(&text) {
        Ok(v) => Some(v),
        Err(e) => {
            out.push(
                Finding::problem(
                    Level::Error,
                    "hooks-file-invalid",
                    format!("{label}: {} isn't valid JSON", path.display()),
                )
                .with_detail(e.to_string()),
            );
            None
        }
    }
}

/// The program a hook command of ours runs, where it names one by its path:
/// its first word (quoted or not), unless that's a shell's.
fn program_of(command: &str) -> Option<PathBuf> {
    let command = command.trim_start();
    let first = if let Some(rest) = command.strip_prefix('"') {
        rest.split('"').next()?
    } else if let Some(rest) = command.strip_prefix('\'') {
        rest.split('\'').next()?
    } else if let Some(rest) = command.strip_prefix("& \"") {
        // PowerShell's call operator (Cursor on Windows).
        rest.split('"').next()?
    } else {
        command.split_whitespace().next()?
    };
    let path = PathBuf::from(first);
    // A leading `/` is rooted but not absolute on Windows (no drive letter);
    // hook commands may still name a path that way.
    (path.is_absolute() || first.starts_with('/')).then_some(path)
}

/// Checks that the hook commands that are ours (`ours`) run a program
/// that's there.
fn programs_there(label: &str, file: &Value, marker: &str, out: &mut Vec<Finding>) {
    for command in commands(file).into_iter().filter(|c| c.contains(marker)) {
        if let Some(program) = program_of(command) {
            if !program.is_file() {
                out.push(Finding::problem(
                    Level::Error,
                    "hook-program-missing",
                    format!(
                        "{label}'s hooks run {}, which isn't there",
                        program.display()
                    ),
                ));
                return;
            }
        } else if let Some(name) = command
            .split_whitespace()
            .next()
            .filter(|n| *n == "agent-graph")
        {
            if crate::paths::find_program(name).is_none() {
                out.push(Finding::problem(
                    Level::Error,
                    "hook-program-missing",
                    format!("{label}'s hooks run agent-graph from PATH, and it isn't on PATH"),
                ));
                return;
            }
        }
    }
}

/// Whether Cursor can read every command in a hooks file (see
/// `install::cursor_can_read`): any one it can't stops every hook in it.
fn cursor_readable(path: &Path, file: &Value, out: &mut Vec<Finding>) -> bool {
    match commands(file)
        .into_iter()
        .find(|c| !install::cursor_can_read(c))
    {
        Some(command) => {
            out.push(
                Finding::problem(
                    Level::Error,
                    "cursor-hooks-comment",
                    format!(
                        "Cursor runs none of the hooks in {}: a command has `//` or `/*` in it",
                        path.display()
                    ),
                )
                .with_detail(crate::event::truncate_chars(command, 160)),
            );
            false
        }
        None => true,
    }
}

/// The hooks for each agent on this computer, in its own settings.
fn local_hooks(project: &Path, out: &mut Vec<Finding>) {
    let Some(home) = crate::paths::user_home() else {
        return;
    };
    let mut any = false;

    if home.join(".claude").is_dir() {
        any = true;
        if let Some(path) = install::claude_settings_path(Scope::User, project) {
            let settings = read_json(&path, "Claude Code", out);
            match settings.as_ref().map(install::our_events) {
                Some(events) if !events.is_empty() => {
                    out.push(Finding::ok(format!(
                        "Claude Code: {} hooks in {}",
                        events.len(),
                        path.display()
                    )));
                    programs_there(
                        "Claude Code",
                        settings.as_ref().unwrap(),
                        "emit --provider claude-code",
                        out,
                    );
                }
                _ if settings.is_some() || !path.exists() => out.push(Finding::problem(
                    Level::Warn,
                    "hooks-missing",
                    "Claude Code is installed, and its sessions aren't recorded: no hooks",
                )),
                _ => {}
            }
        }
    }

    if home.join(".codex").is_dir() {
        any = true;
        if let Some(path) = install::codex_hooks_path(Scope::User, project) {
            let hooks = read_json(&path, "Codex", out);
            match hooks.as_ref().map(install::our_codex_events) {
                Some(events) if !events.is_empty() => {
                    out.push(Finding::ok(format!(
                        "Codex: {} hooks in {}",
                        events.len(),
                        path.display()
                    )));
                    let hooks = hooks.as_ref().unwrap();
                    programs_there("Codex", hooks, "emit --provider codex", out);
                    codex_trusted(&path, hooks, out);
                }
                _ if hooks.is_some() || !path.exists() => out.push(Finding::problem(
                    Level::Warn,
                    "hooks-missing",
                    "Codex is installed, and its sessions aren't recorded: no hooks",
                )),
                _ => {}
            }
        }
    }

    if home.join(".cursor").is_dir() {
        any = true;
        if let Some(path) = install::cursor_hooks_path(Scope::User, project) {
            let hooks = read_json(&path, "Cursor", out);
            if let Some(hooks) = &hooks {
                cursor_readable(&path, hooks, out);
            }
            match hooks.as_ref().map(install::our_cursor_events) {
                Some(events) if !events.is_empty() => {
                    out.push(Finding::ok(format!(
                        "Cursor: {} hooks in {}",
                        events.len(),
                        path.display()
                    )));
                    programs_there(
                        "Cursor",
                        hooks.as_ref().unwrap(),
                        "emit --provider cursor",
                        out,
                    );
                }
                _ if hooks.is_some() || !path.exists() => out.push(Finding::problem(
                    Level::Warn,
                    "hooks-missing",
                    "Cursor is installed, and its chats aren't recorded: no hooks",
                )),
                _ => {}
            }
        }
    }

    if !any {
        out.push(Finding::info(
            "No coding agent Agent Graph records (Claude Code, Codex, Cursor) is set up in this home folder",
        ));
    }
}

/// Whether Codex trusts our hooks in `path`: its `config.toml` already says
/// what installing would.
fn codex_trusted(path: &Path, hooks: &Value, out: &mut Vec<Finding>) {
    let Some(config_path) = install::codex_config_path() else {
        return;
    };
    let config = std::fs::read_to_string(&config_path).unwrap_or_default();
    let source = crate::cli::as_codex_sees(path).display().to_string();
    match install::codex_trust(&config, &source, hooks, hooks) {
        Ok(trusted) if trusted == config => {}
        Ok(_) => out.push(Finding::problem(
            Level::Error,
            "codex-hooks-untrusted",
            format!(
                "Codex doesn't trust Agent Graph's hooks, in {}",
                config_path.display()
            ),
        )),
        Err(e) => out.push(Finding::problem(Level::Warn, "codex-hooks-untrusted", e)),
    }
}

/// A project's committed hooks, in `project` (or the repository it's in):
/// the cloud's, which a cloud needs, and Cursor's, which it may not be able
/// to read.
fn project_hooks(project: &Path, cloud: Option<&str>, out: &mut Vec<Finding>) {
    let root = repository_root(project).unwrap_or_else(|| project.to_path_buf());
    let cursor = root.join(".cursor").join("hooks.json");
    if let Some(hooks) = read_json(&cursor, "This project's Cursor hooks", out) {
        let readable = cursor_readable(&cursor, &hooks, out);
        let ours = install::our_cursor_events(&hooks);
        if cloud == Some(crate::account::CURSOR_CLOUD) {
            if ours.is_empty() {
                out.push(Finding::problem(
                    Level::Error,
                    "hooks-missing",
                    format!("{} has no Agent Graph hooks", cursor.display()),
                ));
            } else if readable {
                out.push(Finding::ok(format!(
                    "{} has the cloud's hooks",
                    cursor.display()
                )));
            }
        }
    } else if cloud == Some(crate::account::CURSOR_CLOUD) {
        out.push(Finding::problem(
            Level::Error,
            "hooks-missing",
            format!("There's no {}", cursor.display()),
        ));
    }

    if cloud == Some(crate::account::CLAUDE_CODE_CLOUD) {
        let settings = root.join(".claude").join("settings.json");
        match read_json(&settings, "This project's Claude Code settings", out) {
            Some(s) if !install::our_events(&s).is_empty() => out.push(Finding::ok(format!(
                "{} has the cloud's hooks",
                settings.display()
            ))),
            Some(_) => out.push(Finding::problem(
                Level::Error,
                "hooks-missing",
                format!("{} has no Agent Graph hooks", settings.display()),
            )),
            None if !settings.exists() => out.push(Finding::problem(
                Level::Error,
                "hooks-missing",
                format!("There's no {}", settings.display()),
            )),
            None => {}
        }
    }
}

/// Whether the commit a cloud agent started on is its branch's latest, as
/// the repository's remote has it: a cloud may start from an older copy of
/// the repository, without the hooks committed since (found in Cursor's).
/// An agent on a branch of its own, not on the remote, is compared with
/// the remote's default branch. Only a warning: starting from another
/// commit may be meant. Says nothing if the remote can't be asked.
fn checkout_current(project: &Path, out: &mut Vec<Finding>) {
    let Some(root) = repository_root(project) else {
        return;
    };
    let git = |args: &[&str]| git_output(&root, args);
    let Some(head) = git(&["rev-parse", "HEAD"]) else {
        return;
    };
    let branch = git(&["symbolic-ref", "--quiet", "--short", "HEAD"]);
    let remote = |r: &str| {
        git(&["ls-remote", "origin", r]).and_then(|o| o.split_whitespace().next().map(String::from))
    };
    let (latest, of) = match branch
        .as_deref()
        .and_then(|b| remote(&format!("refs/heads/{b}")).map(|c| (c, b.to_string())))
    {
        Some(found) => found,
        None => match remote("HEAD") {
            Some(c) => (c, "the default branch".to_string()),
            None => return,
        },
    };
    // Up to date, or ahead: the latest is in what's checked out.
    let has = latest == head
        || (git(&["cat-file", "-e", &format!("{latest}^{{commit}}")]).is_some()
            && git(&["merge-base", "--is-ancestor", &latest, "HEAD"]).is_some());
    let short = |c: &str| c.chars().take(7).collect::<String>();
    if has {
        out.push(Finding::ok(format!(
            "{} has {of}'s latest commit ({})",
            root.display(),
            short(&latest)
        )));
    } else {
        out.push(Finding::problem(
            Level::Warn,
            "checkout-behind",
            format!(
                "{} is on {}, not {of}'s latest commit ({}): hooks committed since aren't here",
                root.display(),
                short(&head),
                short(&latest)
            ),
        ));
    }
}

/// What `git <args>` says in `dir`, trimmed, if it succeeds within 15
/// seconds, never asking for a password.
fn git_output(dir: &Path, args: &[&str]) -> Option<String> {
    let mut child = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        match child.try_wait().ok()? {
            Some(status) if status.success() => break,
            Some(_) => return None,
            None if std::time::Instant::now() > deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    }
    let mut text = String::new();
    std::io::Read::read_to_string(&mut child.stdout.take()?, &mut text).ok()?;
    Some(text.trim().to_string())
}

/// The repository `dir` is in: the nearest folder up with a `.git`.
fn repository_root(dir: &Path) -> Option<PathBuf> {
    dir.ancestors()
        .find(|d| d.join(".git").exists())
        .map(Path::to_path_buf)
}

/// Codex's cloud: the hooks its setup script installs, in its own home.
fn codex_cloud_hooks(out: &mut Vec<Finding>) {
    let Some(path) = install::codex_hooks_path(Scope::User, Path::new(".")) else {
        return;
    };
    match read_json(&path, "Codex", out) {
        Some(h) if !install::our_codex_events(&h).is_empty() => out.push(Finding::ok(format!(
            "Codex's hooks are in {}",
            path.display()
        ))),
        Some(_) | None => out.push(Finding::problem(
            Level::Error,
            "hooks-missing",
            format!("There are no Agent Graph hooks in {}", path.display()),
        )),
    }
}

/// Whether the site answers. In a cloud, sharing is the point, so not
/// reaching it is a problem; on a computer, recording goes on without it.
fn site(client: &Client, site: &str, cloud: Option<&str>, out: &mut Vec<Finding>) -> bool {
    let host = site
        .trim_end_matches('/')
        .split("://")
        .nth(1)
        .unwrap_or(site)
        .to_string();
    let level = if cloud.is_some() {
        Level::Error
    } else {
        Level::Warn
    };
    let n = u64::from(std::process::id()) ^ 0x5eed;
    match client.ping(n) {
        Ok(()) => {
            out.push(Finding::ok(format!("{host} can be reached")));
            true
        }
        Err(Ping::Unreachable(e)) => {
            out.push(
                Finding::problem(
                    level,
                    "site-unreachable",
                    format!("{host} can't be reached"),
                )
                .with_detail(e),
            );
            false
        }
        Err(Ping::Wrong(said)) => {
            out.push(
                Finding::problem(
                    level,
                    "site-wrong-answer",
                    format!("Something other than {host} answered for it"),
                )
                .with_detail(said),
            );
            false
        }
    }
}

/// The login sharing uses: `AGENT_GRAPH_TOKEN` (which a cloud needs), or
/// this computer's saved one. Whether there's one the site knows (or might:
/// it couldn't be asked).
fn login(
    client: &Client,
    root: Option<&Path>,
    site: &str,
    cloud: Option<&str>,
    reached: bool,
    out: &mut Vec<Finding>,
) -> bool {
    let var = crate::account::TOKEN_VAR;
    let from_env = std::env::var(var)
        .ok()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty());
    let (token, what) = match (from_env, cloud) {
        (Some(token), _) => (token, format!("{var} (set)")),
        (None, Some(_)) => {
            out.push(Finding::problem(
                Level::Error,
                "token-missing",
                format!("{var} isn't set, so nothing can be shared"),
            ));
            return false;
        }
        (None, None) => match root.and_then(|r| crate::account::load(r, site)) {
            Some(account) => (account.token.clone(), "This computer's login".to_string()),
            None => {
                out.push(
                    Finding::problem(
                        Level::Info,
                        "not-logged-in",
                        "This computer isn't logged in to the site: sessions are recorded, and not shared",
                    ),
                );
                return false;
            }
        },
    };
    if !reached {
        out.push(Finding::info(format!(
            "{what} can't be checked without reaching the site"
        )));
        return true;
    }
    match client.account_email(&token) {
        Ok(email) => {
            out.push(Finding::ok(format!(
                "{what} is a login the site knows{}",
                email.map(|e| format!(", for {e}")).unwrap_or_default()
            )));
            true
        }
        Err(SendError::LoggedOut(_)) => {
            out.push(Finding::problem(
                if cloud.is_some() {
                    Level::Error
                } else {
                    Level::Warn
                },
                "token-invalid",
                format!("{what} isn't a login the site knows: revoked, or mistyped"),
            ));
            false
        }
        Err(e) => {
            out.push(Finding::info(format!(
                "{what} couldn't be checked: {}",
                e.message()
            )));
            true
        }
    }
}

/// Whether a `watch-remote` is sharing now.
fn sharing(root: Option<&Path>, site: &str, cloud: Option<&str>, out: &mut Vec<Finding>) {
    let Some(root) = root else {
        return;
    };
    if crate::remote::sharing_now(root, site) {
        out.push(Finding::ok("agent-graph watch-remote is sharing"));
    } else {
        out.push(Finding::problem(
            if cloud.is_some() {
                Level::Error
            } else {
                Level::Info
            },
            "not-sharing",
            "agent-graph watch-remote isn't running, so nothing new is shared",
        ));
    }
}

/// Whether there's a newer release than this.
fn version(client: &Client, out: &mut Vec<Finding>) {
    let this = env!("CARGO_PKG_VERSION");
    match client.latest_version() {
        Some(latest) if newer(&latest, this) => out.push(Finding::problem(
            Level::Warn,
            "update-available",
            format!("agent-graph {latest} is out (this is {this})"),
        )),
        Some(_) => out.push(Finding::ok(format!("agent-graph {this} is the latest"))),
        None => {}
    }
}

/// Whether version `a` is newer than `b` (by their numbers: a prerelease
/// counts as its release).
fn newer(a: &str, b: &str) -> bool {
    let parts = |v: &str| -> Vec<u64> {
        v.split(['.', '-'])
            .take(3)
            .map(|p| p.parse().unwrap_or(0))
            .collect()
    };
    parts(a) > parts(b)
}

/// The ends of agent-graph's own logs in a cloud (what sharing, and
/// installing, said), with any token hidden; and whether installing failed.
fn logs(root: &Path, cloud: Option<&str>, out: &mut Vec<Finding>) -> BTreeMap<String, String> {
    let mut logs = BTreeMap::new();
    if cloud.is_none() {
        return logs;
    }
    for name in ["watch-remote.log", "install.log"] {
        let Ok(text) = std::fs::read_to_string(root.join(name)) else {
            continue;
        };
        let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        let tail = lines[lines.len().saturating_sub(20)..].join("\n");
        if tail.is_empty() {
            continue;
        }
        if name == "install.log" && !tail.contains("Installed ") {
            out.push(
                Finding::problem(
                    Level::Error,
                    "install-failed",
                    "Installing agent-graph here failed",
                )
                .with_detail(crate::event::truncate_chars(
                    lines.last().unwrap_or(&""),
                    200,
                )),
            );
        }
        logs.insert(name.to_string(), redact(&tail));
    }
    logs
}

/// `text` with any API token (`agt_…`) or login secret hidden.
pub fn redact(text: &str) -> String {
    let mut text = text.to_string();
    if let Some(token) = std::env::var(crate::account::TOKEN_VAR)
        .ok()
        .map(|t| t.trim().to_string())
        .filter(|t| t.len() >= 8)
    {
        text = text.replace(&token, "(token hidden)");
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text.as_str();
    while let Some(at) = rest.find("agt_") {
        out.push_str(&rest[..at]);
        let after = &rest[at + 4..];
        let len = after
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
            .unwrap_or(after.len());
        out.push_str(if len >= 8 {
            "agt_(hidden)"
        } else {
            &rest[at..at + 4 + len]
        });
        rest = &after[len..];
    }
    out.push_str(rest);
    out
}

/// In a cloud: checks everything and sends the report to the site, as
/// the account whose login `AGENT_GRAPH_TOKEN` is (if it's one the site
/// knows), with `stopped`, the error sharing stopped with, if it did. A
/// report with no problems clears that cloud's earlier ones at /watch.
/// Whatever goes wrong sending it is only said.
pub fn report_from_cloud(site: &str, stopped: Option<&str>) {
    if crate::account::cloud().is_none() {
        return;
    }
    let project = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut report = run(&Options {
        site: site.to_string(),
        project,
        from_sharer: true,
    });
    if let Some(error) = stopped {
        report.findings.push(
            Finding::problem(Level::Error, "sharing-stopped", "Sharing stopped")
                .with_detail(redact(error)),
        );
    }
    if let Err(e) = send(site, &report) {
        eprintln!("Couldn't send the diagnostics report to {site}: {e}");
    }
}

/// Sends `report` to `site`, as `AGENT_GRAPH_TOKEN`'s account.
pub fn send(site: &str, report: &Report) -> Result<(), String> {
    let token = std::env::var(crate::account::TOKEN_VAR)
        .ok()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .ok_or_else(|| format!("{} isn't set", crate::account::TOKEN_VAR))?;
    let body = serde_json::to_vec(report).map_err(|e| e.to_string())?;
    Client::new(site)
        .report_diagnostics(&token, &body)
        .map_err(|e| e.message())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every issue the checks name is in the catalogue (and so on the site).
    #[test]
    fn every_issue_is_explained() {
        let source = include_str!("diagnostics.rs");
        let checks = source.split("#[cfg(test)]").next().unwrap();
        let mut named = 0;
        for chunk in checks.split("Finding::problem(").skip(1) {
            let id = chunk
                .split('"')
                .nth(1)
                .expect("each problem names its issue");
            assert!(
                issue(id).is_some(),
                "{id} isn't in docs/troubleshooting.json"
            );
            named += 1;
        }
        assert!(named > 10);
        for i in serde_json::from_str::<Catalogue>(CATALOGUE).unwrap().issues {
            assert!(
                i.id.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
                "{}",
                i.id
            );
            for cloud in i.clouds.keys() {
                assert!(
                    [
                        crate::account::CLAUDE_CODE_CLOUD,
                        crate::account::CODEX_CLOUD,
                        crate::account::CURSOR_CLOUD
                    ]
                    .contains(&cloud.as_str()),
                    "{}: {cloud}",
                    i.id
                );
            }
        }
    }

    #[test]
    fn tokens_never_show() {
        assert_eq!(
            redact("logged in with agt_AbCdEfGhIjKl_mn-op, then"),
            "logged in with agt_(hidden), then"
        );
        assert_eq!(redact("agt_ short"), "agt_ short");
    }

    #[test]
    fn hook_programs_are_found_in_their_commands() {
        assert_eq!(
            program_of("\"/Users/a b/bin/agent-graph\" emit --provider cursor"),
            Some(PathBuf::from("/Users/a b/bin/agent-graph"))
        );
        assert_eq!(
            program_of("/opt/agent-graph emit --provider codex"),
            Some(PathBuf::from("/opt/agent-graph"))
        );
        assert_eq!(program_of("agent-graph emit --provider codex"), None);
    }

    #[test]
    fn versions_compare_by_their_numbers() {
        assert!(newer("0.1.21", "0.1.20"));
        assert!(newer("0.2.0", "0.1.30"));
        assert!(!newer("0.1.20", "0.1.20"));
        assert!(!newer("0.1.9", "0.1.20"));
    }

    #[test]
    fn a_cursor_file_with_a_url_in_a_command_is_a_problem() {
        let mut out = Vec::new();
        let file = serde_json::json!({"version": 1, "hooks": {"stop": [
            {"command": "curl https://example.com/x"}
        ]}});
        assert!(!cursor_readable(Path::new("hooks.json"), &file, &mut out));
        assert_eq!(out[0].issue.as_deref(), Some("cursor-hooks-comment"));
    }

    #[test]
    fn a_report_is_the_same_breakage_by_its_errors() {
        let report = Report {
            version: "0".into(),
            cloud: Some("Cursor cloud".into()),
            platform: "linux x86_64".into(),
            site: "https://x".into(),
            at: "now".into(),
            findings: vec![
                Finding::problem(Level::Error, "token-missing", "a"),
                Finding::problem(Level::Warn, "update-available", "b"),
                Finding::problem(Level::Error, "hooks-missing", "c"),
                Finding::problem(Level::Error, "token-missing", "d"),
            ],
            logs: BTreeMap::new(),
        };
        assert_eq!(report.errors(), ["hooks-missing", "token-missing"]);
        let text = report.text();
        assert!(text.contains("troubleshooting#token-missing"), "{text}");
        assert!(text.contains("Cloud Agents, then Secrets"), "{text}");
        assert!(text.contains("3 problems and 1 warning."), "{text}");
    }
}
