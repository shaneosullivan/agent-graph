//! `agent-graph install` / `uninstall`: merge our hooks into a provider's
//! settings without disturbing anything else in the file.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use crate::paths;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// `~/.claude/settings.json`: every project.
    User,
    /// `.claude/settings.json`: this project, committed.
    Project,
    /// `.claude/settings.local.json`: this project, not committed.
    Local,
}

/// One hook registration.
struct HookSpec {
    event: &'static str,
    matcher: Option<&'static str>,
    /// Run in the background so the agent never waits for us.
    background: bool,
}

/// The Claude Code hooks Agent Graph needs.
///
/// Tool hooks only match the tools that matter to the graph, because every
/// match starts a process. `SessionStart` stays synchronous so it's recorded
/// before anything else in the session.
const CLAUDE_CODE_HOOKS: &[HookSpec] = &[
    HookSpec {
        event: "SessionStart",
        matcher: None,
        background: false,
    },
    HookSpec {
        event: "SessionEnd",
        matcher: None,
        background: true,
    },
    HookSpec {
        event: "UserPromptSubmit",
        matcher: None,
        background: true,
    },
    HookSpec {
        event: "Stop",
        matcher: None,
        background: true,
    },
    HookSpec {
        event: "Notification",
        matcher: None,
        background: true,
    },
    HookSpec {
        event: "SubagentStart",
        matcher: None,
        background: true,
    },
    HookSpec {
        event: "SubagentStop",
        matcher: None,
        background: true,
    },
    // Bash: to see a session start another agent from its shell. For any
    // other command, `emit` records nothing.
    HookSpec {
        event: "PreToolUse",
        matcher: Some("Agent|Task|AskUserQuestion|ExitPlanMode|Bash"),
        background: true,
    },
    HookSpec {
        event: "PostToolUse",
        matcher: Some(
            "Agent|Task|TaskCreate|TaskUpdate|TodoWrite|SendMessage|AskUserQuestion|ExitPlanMode|Bash",
        ),
        background: true,
    },
];

/// Identifies our handlers, whatever path the command starts with.
pub const CLAUDE_CODE_MARKER: &str = "emit --provider claude-code";

/// Seconds Claude Code waits for the synchronous hook.
const SYNC_TIMEOUT_SECS: u64 = 10;

/// Also ours: the hook that shares from Claude Code's cloud (see
/// `install_claude_code_cloud`), which runs no `emit`.
const CLOUD_SHARE_MARKER: &str = "watch-remote --background";

/// Where the cloud hooks install `agent-graph`, and run it from.
const CLOUD_BIN: &str = "\"$HOME/.local/bin/agent-graph\"";

/// Seconds Claude Code waits for the cloud's `SessionStart` hook, which may
/// first download and install `agent-graph`.
const CLOUD_START_TIMEOUT_SECS: u64 = 180;

/// Each cloud hook starts with this: in Claude Code's cloud (claude.ai/code)
/// it carries on; anywhere else it does nothing, so the same committed
/// settings never record a session a second time on a computer that
/// records its own.
const IN_CLOUD: &str = "[ \"$CLAUDE_CODE_REMOTE\" = true ] || exit 0";

/// The cloud's `SessionStart` hook: installs `agent-graph` (from the site's
/// install script, where the cloud's setup hasn't), then records the start.
fn cloud_start_command(site: &str) -> String {
    format!(
        "{IN_CLOUD}; [ -x {CLOUD_BIN} ] || curl -fsSL {site}/install.sh | \
         AGENT_GRAPH_INSTALL_DIR=\"$HOME/.local/bin\" sh >/dev/null 2>&1; \
         {CLOUD_BIN} {CLAUDE_CODE_MARKER}; exit 0"
    )
}

/// The cloud's other hooks: record the event, once `agent-graph`'s there.
fn cloud_emit_command() -> String {
    format!("{IN_CLOUD}; [ -x {CLOUD_BIN} ] && {CLOUD_BIN} {CLAUDE_CODE_MARKER}; exit 0")
}

/// The cloud's second `SessionStart` hook, in the background: once
/// `agent-graph`'s installed, shares to the account whose API token the
/// environment gives (AGENT_GRAPH_TOKEN), for as long as the session lasts.
fn cloud_share_command(site: &str) -> String {
    let url = if site.trim_end_matches('/') == crate::remote::DEFAULT_URL {
        String::new()
    } else {
        format!(" --url={site}")
    };
    format!(
        "{IN_CLOUD}; for i in $(seq 180); do [ -x {CLOUD_BIN} ] && break; sleep 1; done; \
         exec {CLOUD_BIN} {CLOUD_SHARE_MARKER}{url}"
    )
}

pub fn claude_settings_path(scope: Scope, project_dir: &Path) -> Option<PathBuf> {
    Some(match scope {
        Scope::User => std::env::var_os("CLAUDE_CONFIG_DIR")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| paths::user_home().map(|h| h.join(".claude")))?
            .join("settings.json"),
        Scope::Project => project_dir.join(".claude").join("settings.json"),
        Scope::Local => project_dir.join(".claude").join("settings.local.json"),
    })
}

/// The hook command that runs `agent-graph` from PATH, for settings shared
/// with others (project scope, which is committed): this machine's path to
/// it means nothing on anyone else's. (If you also have user-scope hooks,
/// whose command is this copy's full path, both run; the reducer counts
/// the repeated events once.)
pub fn path_command(provider: &str) -> String {
    format!("agent-graph emit --provider {provider}")
}

/// Whether the `agent-graph` on PATH is this executable, or `None` if
/// there isn't one.
pub fn this_is_on_path() -> Option<bool> {
    let found = paths::find_program("agent-graph")?;
    let this = std::env::current_exe().ok()?;
    Some(same_file(&found, &this))
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// The path hooks should run this executable by. A package manager keeps
/// the program in a folder named for its version, which an upgrade
/// removes, and links to it from a folder on PATH that stays put
/// (Homebrew's `bin`, WinGet's `Links`): when the `agent-graph` on PATH is
/// this copy, that path, as PATH names it, not the file it leads to.
pub fn lasting_exe() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("can't find this executable: {e}"))?;
    Ok(match paths::find_program("agent-graph") {
        Some(found) if same_file(&found, &exe) => found,
        _ => exe,
    })
}

/// The hook command pointing at this executable (`lasting_exe`). Forward
/// slashes and quotes keep it working in the shells Claude Code uses on
/// every platform.
pub fn default_command(provider: &str) -> Result<String, String> {
    let exe = lasting_exe()?.to_string_lossy().replace('\\', "/");
    Ok(format!("\"{exe}\" emit --provider {provider}"))
}

/// Adds our hooks to Claude Code `settings`, replacing any earlier copy.
/// `command` must contain `CLAUDE_CODE_MARKER`, which is how they're found
/// again: otherwise each install would add them again, and uninstall
/// would leave them.
pub fn install_claude_code(settings: &mut Value, command: &str) -> Result<(), String> {
    if !command.contains(CLAUDE_CODE_MARKER) {
        return Err(format!(
            "the hook command must contain `{CLAUDE_CODE_MARKER}`, which is how Agent Graph \
             finds its hooks again (to replace or remove them), so nothing was changed: \
             {command}"
        ));
    }
    uninstall_claude_code(settings)?;
    let hooks = hooks_object(settings)?;
    for spec in CLAUDE_CODE_HOOKS {
        let mut handler = Map::new();
        handler.insert("type".into(), json!("command"));
        handler.insert("command".into(), json!(command));
        if spec.background {
            handler.insert("async".into(), json!(true));
        } else {
            handler.insert("timeout".into(), json!(SYNC_TIMEOUT_SECS));
        }
        let mut group = Map::new();
        if let Some(matcher) = spec.matcher {
            group.insert("matcher".into(), json!(matcher));
        }
        group.insert("hooks".into(), json!([handler]));

        let groups = hooks.entry(spec.event).or_insert_with(|| json!([]));
        let groups = groups
            .as_array_mut()
            .ok_or_else(|| format!("settings.hooks.{} isn't a list", spec.event))?;
        groups.push(Value::Object(group));
    }
    Ok(())
}

/// Adds our hooks for Claude Code's cloud (claude.ai/code) to a project's
/// Claude Code `settings`, sharing to `site`, replacing any earlier copy.
/// They're committed, and do nothing but in the cloud (see `IN_CLOUD`).
/// There, `SessionStart` installs `agent-graph` if it isn't there (no setup
/// script needed), and records the start; a second, in the background,
/// shares the session live with the API token the environment gives as
/// AGENT_GRAPH_TOKEN; and the rest record each event, as the hooks on a
/// computer do.
pub fn install_claude_code_cloud(settings: &mut Value, site: &str) -> Result<(), String> {
    uninstall_claude_code(settings)?;
    let hooks = hooks_object(settings)?;
    for spec in CLAUDE_CODE_HOOKS {
        let mut handlers = Vec::new();
        if spec.event == "SessionStart" {
            handlers.push(json!({
                "type": "command",
                "command": cloud_start_command(site),
                "timeout": CLOUD_START_TIMEOUT_SECS,
            }));
            handlers.push(json!({
                "type": "command",
                "command": cloud_share_command(site),
                "async": true,
            }));
        } else {
            let mut handler = Map::new();
            handler.insert("type".into(), json!("command"));
            handler.insert("command".into(), json!(cloud_emit_command()));
            if spec.background {
                handler.insert("async".into(), json!(true));
            } else {
                handler.insert("timeout".into(), json!(SYNC_TIMEOUT_SECS));
            }
            handlers.push(Value::Object(handler));
        }
        let mut group = Map::new();
        if let Some(matcher) = spec.matcher {
            group.insert("matcher".into(), json!(matcher));
        }
        group.insert("hooks".into(), json!(handlers));
        let groups = hooks.entry(spec.event).or_insert_with(|| json!([]));
        let groups = groups
            .as_array_mut()
            .ok_or_else(|| format!("settings.hooks.{} isn't a list", spec.event))?;
        groups.push(Value::Object(group));
    }
    Ok(())
}

/// Removes our hooks from Claude Code `settings`. Returns how many handlers
/// were removed. Other handlers, and groups that still have any, are kept.
pub fn uninstall_claude_code(settings: &mut Value) -> Result<usize, String> {
    remove_handlers(settings, is_ours)
}

/// Removes the handlers `ours` picks from a hooks file (Claude Code's or
/// Codex's, which have the same shape). Other handlers, and groups that
/// still have any, are kept.
fn remove_handlers(settings: &mut Value, ours: fn(&Value) -> bool) -> Result<usize, String> {
    let Some(hooks) = settings.get_mut("hooks") else {
        return Ok(0);
    };
    let hooks = hooks
        .as_object_mut()
        .ok_or("settings.hooks isn't an object")?;
    let mut removed = 0;
    let mut emptied = Vec::new();
    for (event, groups) in hooks.iter_mut() {
        let Some(groups) = groups.as_array_mut() else {
            continue;
        };
        let mut removed_here = 0;
        groups.retain_mut(|group| {
            let Some(handlers) = group.get_mut("hooks").and_then(Value::as_array_mut) else {
                return true;
            };
            let before = handlers.len();
            handlers.retain(|h| !ours(h));
            removed_here += before - handlers.len();
            // Drop a group only if we emptied it.
            !(handlers.is_empty() && before > 0)
        });
        removed += removed_here;
        if removed_here > 0 && groups.is_empty() {
            emptied.push(event.clone());
        }
    }
    for event in emptied {
        hooks.shift_remove(&event);
    }
    if hooks.is_empty() && removed > 0 {
        settings
            .as_object_mut()
            .expect("has hooks, so is an object")
            .shift_remove("hooks");
    }
    Ok(removed)
}

fn is_ours(handler: &Value) -> bool {
    handler
        .get("command")
        .and_then(Value::as_str)
        .is_some_and(|c| c.contains(CLAUDE_CODE_MARKER) || c.contains(CLOUD_SHARE_MARKER))
}

fn hooks_object(settings: &mut Value) -> Result<&mut Map<String, Value>, String> {
    let settings = settings
        .as_object_mut()
        .ok_or("settings file isn't a JSON object")?;
    settings
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| "settings.hooks isn't an object".to_string())
}

/// The events that have one of our handlers, for showing what changed.
pub fn our_events(settings: &Value) -> Vec<String> {
    events_with(settings, is_ours)
}

fn events_with(settings: &Value, ours: fn(&Value) -> bool) -> Vec<String> {
    let Some(hooks) = settings.get("hooks").and_then(Value::as_object) else {
        return Vec::new();
    };
    hooks
        .iter()
        .filter(|(_, groups)| {
            groups.as_array().is_some_and(|groups| {
                groups.iter().any(|g| {
                    g.get("hooks")
                        .and_then(Value::as_array)
                        .is_some_and(|hs| hs.iter().any(ours))
                })
            })
        })
        .map(|(event, _)| event.clone())
        .collect()
}

// ---------------------------------------------------------------------------
// Codex (docs/codex.md)

/// One Codex hook registration.
struct CodexHook {
    event: &'static str,
    matcher: Option<&'static str>,
    background: bool,
    timeout: u64,
}

/// The tools the graph needs to hear about: shell commands (to see another
/// agent started, and approvals given), edits (approvals), subagents, the
/// plan, and questions for you. `wait_agent` is named with its namespace
/// run into it (`multi_agent_v1wait_agent`). Plain names match exactly.
const CODEX_TOOLS: &str = "Bash|apply_patch|spawn_agent|wait_agent|multi_agent_v1wait_agent|update_plan|request_user_input";

/// The Codex hooks Agent Graph needs. SessionStart is synchronous, so it's
/// recorded before anything else in the session, and so is SessionEnd,
/// which Codex always runs synchronously (for at most 3 s).
const CODEX_HOOKS: &[CodexHook] = &[
    CodexHook {
        event: "SessionStart",
        matcher: None,
        background: false,
        timeout: SYNC_TIMEOUT_SECS,
    },
    CodexHook {
        event: "SessionEnd",
        matcher: None,
        background: false,
        timeout: 3,
    },
    CodexHook {
        event: "UserPromptSubmit",
        matcher: None,
        background: true,
        timeout: SYNC_TIMEOUT_SECS,
    },
    CodexHook {
        event: "Stop",
        matcher: None,
        background: true,
        timeout: SYNC_TIMEOUT_SECS,
    },
    CodexHook {
        event: "Interrupt",
        matcher: None,
        background: true,
        timeout: 3,
    },
    CodexHook {
        event: "SubagentStart",
        matcher: None,
        background: true,
        timeout: SYNC_TIMEOUT_SECS,
    },
    CodexHook {
        event: "SubagentStop",
        matcher: None,
        background: true,
        timeout: SYNC_TIMEOUT_SECS,
    },
    CodexHook {
        event: "PermissionRequest",
        matcher: None,
        background: true,
        timeout: SYNC_TIMEOUT_SECS,
    },
    CodexHook {
        event: "PreToolUse",
        matcher: Some(CODEX_TOOLS),
        background: true,
        timeout: SYNC_TIMEOUT_SECS,
    },
    CodexHook {
        event: "PostToolUse",
        matcher: Some(CODEX_TOOLS),
        background: true,
        timeout: SYNC_TIMEOUT_SECS,
    },
];

/// Identifies our Codex handlers, whatever path the command starts with.
pub const CODEX_MARKER: &str = "emit --provider codex";

/// Codex's hooks file: `$CODEX_HOME/hooks.json` (`~/.codex`), or a
/// project's `.codex/hooks.json`. Codex has no file of a project's that
/// isn't committed, so there's none for `Scope::Local`.
pub fn codex_hooks_path(scope: Scope, project_dir: &Path) -> Option<PathBuf> {
    match scope {
        Scope::User => Some(crate::adapter::codex::codex_home()?.join("hooks.json")),
        Scope::Project => Some(project_dir.join(".codex").join("hooks.json")),
        Scope::Local => None,
    }
}

/// Codex's own settings, where hooks are trusted: `$CODEX_HOME/config.toml`.
pub fn codex_config_path() -> Option<PathBuf> {
    Some(crate::adapter::codex::codex_home()?.join("config.toml"))
}

/// Adds our hooks to a Codex hooks file, replacing any earlier copy.
/// `command` must contain `CODEX_MARKER`.
pub fn install_codex(hooks_file: &mut Value, command: &str) -> Result<(), String> {
    if !command.contains(CODEX_MARKER) {
        return Err(format!(
            "the hook command must contain `{CODEX_MARKER}`, which is how Agent Graph \
             finds its hooks again (to replace or remove them), so nothing was changed: \
             {command}"
        ));
    }
    uninstall_codex(hooks_file)?;
    let hooks = hooks_object(hooks_file)?;
    for spec in CODEX_HOOKS {
        let mut handler = Map::new();
        handler.insert("type".into(), json!("command"));
        handler.insert("command".into(), json!(command));
        handler.insert("timeout".into(), json!(spec.timeout));
        if spec.background {
            handler.insert("async".into(), json!(true));
        }
        let mut group = Map::new();
        if let Some(matcher) = spec.matcher {
            group.insert("matcher".into(), json!(matcher));
        }
        group.insert("hooks".into(), json!([handler]));
        let groups = hooks.entry(spec.event).or_insert_with(|| json!([]));
        let groups = groups
            .as_array_mut()
            .ok_or_else(|| format!("hooks.{} isn't a list", spec.event))?;
        groups.push(Value::Object(group));
    }
    Ok(())
}

/// Removes our hooks from a Codex hooks file. Returns how many handlers
/// were removed.
pub fn uninstall_codex(hooks_file: &mut Value) -> Result<usize, String> {
    remove_handlers(hooks_file, is_ours_codex)
}

fn is_ours_codex(handler: &Value) -> bool {
    handler
        .get("command")
        .and_then(Value::as_str)
        .is_some_and(|c| c.contains(CODEX_MARKER))
}

/// The events with one of our Codex handlers.
pub fn our_codex_events(hooks_file: &Value) -> Vec<String> {
    events_with(hooks_file, is_ours_codex)
}

/// How Codex names an event in its trust keys, for those it has.
fn codex_label(event: &str) -> Option<&'static str> {
    Some(match event {
        "PreToolUse" => "pre_tool_use",
        "PermissionRequest" => "permission_request",
        "PostToolUse" => "post_tool_use",
        "PreCompact" => "pre_compact",
        "PostCompact" => "post_compact",
        "SessionStart" => "session_start",
        "SessionEnd" => "session_end",
        "UserPromptSubmit" => "user_prompt_submit",
        "SubagentStart" => "subagent_start",
        "SubagentStop" => "subagent_stop",
        "Stop" => "stop",
        "Interrupt" => "interrupt",
        _ => return None,
    })
}

/// A handler in a Codex hooks file: where it is, as Codex's trust keys
/// place it, and whether it's ours.
struct Placed<'a> {
    label: &'static str,
    group: usize,
    index: usize,
    matcher: Option<&'a str>,
    handler: &'a Value,
    ours: bool,
}

fn placed(hooks_file: &Value) -> Vec<Placed<'_>> {
    let mut out = Vec::new();
    let Some(hooks) = hooks_file.get("hooks").and_then(Value::as_object) else {
        return out;
    };
    for (event, groups) in hooks {
        let (Some(label), Some(groups)) = (codex_label(event), groups.as_array()) else {
            continue;
        };
        for (g, group) in groups.iter().enumerate() {
            let matcher = group.get("matcher").and_then(Value::as_str);
            let Some(handlers) = group.get("hooks").and_then(Value::as_array) else {
                continue;
            };
            for (h, handler) in handlers.iter().enumerate() {
                out.push(Placed {
                    label,
                    group: g,
                    index: h,
                    matcher,
                    handler,
                    ours: is_ours_codex(handler),
                });
            }
        }
    }
    out
}

/// Codex's trust key for a handler in the hooks file at `source`.
fn trust_key(source: &str, p: &Placed) -> String {
    format!("{source}:{}:{}:{}", p.label, p.group, p.index)
}

/// The hash Codex trusts one of our handlers by: SHA-256 over the handler,
/// as Codex normalizes it (its timeout filled in and clamped; the matcher
/// only for events that take one), as compact JSON with sorted keys.
fn codex_trust_hash(label: &str, matcher: Option<&str>, handler: &Value) -> String {
    let (default, max) = match label {
        "session_end" | "interrupt" => (1, Some(3)),
        _ => (600, None),
    };
    let timeout = handler
        .get("timeout")
        .and_then(Value::as_u64)
        .unwrap_or(default)
        .max(1);
    let timeout = max.map_or(timeout, |max: u64| timeout.min(max));
    let mut normalized = json!({
        "type": "command",
        "command": handler.get("command").cloned().unwrap_or(Value::Null),
        "timeout": timeout,
        "async": handler.get("async").and_then(Value::as_bool).unwrap_or(false),
    });
    if let Some(message) = handler.get("statusMessage") {
        normalized["statusMessage"] = message.clone();
    }
    let mut identity = json!({ "event_name": label, "hooks": [normalized] });
    if !matches!(label, "user_prompt_submit" | "stop" | "interrupt") {
        if let Some(matcher) = matcher {
            identity["matcher"] = json!(matcher);
        }
    }
    let bytes = sorted_json(&identity);
    let digest = ring::digest::digest(&ring::digest::SHA256, bytes.as_bytes());
    let hex: String = digest.as_ref().iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256:{hex}")
}

/// Compact JSON with every object's keys sorted.
fn sorted_json(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let fields: Vec<String> = keys
                .into_iter()
                .map(|k| format!("{}:{}", Value::String(k.clone()), sorted_json(&map[k])))
                .collect();
            format!("{{{}}}", fields.join(","))
        }
        Value::Array(items) => {
            let items: Vec<String> = items.iter().map(sorted_json).collect();
            format!("[{}]", items.join(","))
        }
        other => other.to_string(),
    }
}

/// Codex's `config.toml` (as `config`, empty if there's none) with its
/// hook trust brought in line with a hooks file (at `source`, as Codex
/// names it) going from `before` to `after`: our handlers trusted where
/// they now are, and the rest's trust (or anything else Codex keeps for
/// them) moved with them, as our handlers coming or going shifts their
/// places. Everything else in the file is kept as it was.
pub fn codex_trust(
    config: &str,
    source: &str,
    before: &Value,
    after: &Value,
) -> Result<String, String> {
    use toml_edit::{DocumentMut, Item, Table, value};
    let mut doc: DocumentMut = config
        .parse()
        .map_err(|e| format!("Codex's config.toml isn't valid TOML, so it wasn't changed: {e}"))?;
    let root = doc.as_table_mut();
    if !root.contains_key("hooks") {
        let mut hooks = Table::new();
        hooks.set_implicit(true);
        root.insert("hooks", Item::Table(hooks));
    }
    let hooks = root["hooks"]
        .as_table_like_mut()
        .ok_or("hooks in Codex's config.toml isn't a table")?;
    if !hooks.contains_key("state") {
        let mut state = Table::new();
        state.set_implicit(true);
        hooks.insert("state", Item::Table(state));
    }
    let state = hooks
        .get_mut("state")
        .and_then(Item::as_table_like_mut)
        .ok_or("hooks.state in Codex's config.toml isn't a table")?;

    // Take out what's kept for every handler as it was, the rest's in order.
    let mut theirs: std::collections::BTreeMap<&str, std::collections::VecDeque<Option<Item>>> =
        Default::default();
    for p in placed(before) {
        let item = state.remove(&trust_key(source, &p));
        if !p.ours {
            theirs.entry(p.label).or_default().push_back(item);
        }
    }
    // And put it back where each now is.
    for p in placed(after) {
        let key = trust_key(source, &p);
        if p.ours {
            let mut entry = Table::new();
            entry.insert(
                "trusted_hash",
                value(codex_trust_hash(p.label, p.matcher, p.handler)),
            );
            state.insert(&key, Item::Table(entry));
        } else if let Some(Some(item)) = theirs.get_mut(p.label).and_then(|q| q.pop_front()) {
            state.insert(&key, item);
        }
    }
    let state_empty = state.is_empty();
    if state_empty {
        hooks.remove("state");
    }
    if hooks.is_empty() {
        doc.as_table_mut().remove("hooks");
    }
    Ok(doc.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CMD: &str = "\"/usr/local/bin/agent-graph\" emit --provider claude-code";

    #[test]
    fn install_keeps_other_settings_and_hooks() {
        let mut settings = json!({
            "model": "opus",
            "hooks": {
                "Stop": [{ "hooks": [{ "type": "command", "command": "say done" }] }]
            }
        });
        install_claude_code(&mut settings, CMD).unwrap();

        assert_eq!(settings["model"], "opus");
        let stop = settings["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 2);
        assert_eq!(stop[0]["hooks"][0]["command"], "say done");
        assert_eq!(stop[1]["hooks"][0]["command"], CMD);
        assert_eq!(stop[1]["hooks"][0]["async"], true);
        assert_eq!(
            settings["hooks"]["SessionStart"][0]["hooks"][0]["timeout"],
            SYNC_TIMEOUT_SECS
        );
        assert_eq!(
            settings["hooks"]["PreToolUse"][0]["matcher"],
            "Agent|Task|AskUserQuestion|ExitPlanMode|Bash"
        );
        assert_eq!(our_events(&settings).len(), CLAUDE_CODE_HOOKS.len());
    }

    /// A command the hooks can't be found by again would be added again by
    /// each install, and left by uninstall: it's refused.
    #[test]
    fn a_command_that_cant_be_found_again_is_refused() {
        for command in ["my-wrapper", "agent-graph emit --provider=claude-code"] {
            let mut settings = json!({ "model": "opus" });
            let err = install_claude_code(&mut settings, command).unwrap_err();
            assert!(err.contains(CLAUDE_CODE_MARKER), "{err}");
            assert_eq!(settings, json!({ "model": "opus" }), "unchanged");
        }
        let mut settings = json!({});
        install_claude_code(
            &mut settings,
            "nice -n 5 ag emit --provider claude-code --x",
        )
        .unwrap();
        assert_eq!(
            uninstall_claude_code(&mut settings).unwrap(),
            CLAUDE_CODE_HOOKS.len()
        );
    }

    #[test]
    fn the_cloud_hooks_do_nothing_but_in_the_cloud_and_come_out_again() {
        let mut settings = json!({"model": "opus"});
        install_claude_code_cloud(&mut settings, crate::remote::DEFAULT_URL).unwrap();
        let hooks = settings["hooks"].as_object().unwrap();
        // Every command does nothing unless in Claude Code's cloud, and runs
        // the copy it installs there, not one from PATH.
        for groups in hooks.values() {
            for group in groups.as_array().unwrap() {
                for handler in group["hooks"].as_array().unwrap() {
                    let command = handler["command"].as_str().unwrap();
                    assert!(command.starts_with(IN_CLOUD), "{command}");
                    assert!(command.contains(CLOUD_BIN), "{command}");
                }
            }
        }
        // SessionStart installs it, then records; a second shares, in the background.
        let start = hooks["SessionStart"][0]["hooks"].as_array().unwrap();
        assert!(
            start[0]["command"]
                .as_str()
                .unwrap()
                .contains("/install.sh")
        );
        assert!(
            start[0]["command"]
                .as_str()
                .unwrap()
                .contains(CLAUDE_CODE_MARKER)
        );
        assert_eq!(start[0]["timeout"], CLOUD_START_TIMEOUT_SECS);
        assert!(
            start[1]["command"]
                .as_str()
                .unwrap()
                .contains(CLOUD_SHARE_MARKER)
        );
        assert_eq!(start[1]["async"], true);
        // Installed again, it replaces itself; uninstalled, all of it goes.
        let once = settings.clone();
        install_claude_code_cloud(&mut settings, crate::remote::DEFAULT_URL).unwrap();
        assert_eq!(settings, once);
        uninstall_claude_code(&mut settings).unwrap();
        assert_eq!(settings, json!({"model": "opus"}));
    }

    #[test]
    fn install_twice_is_idempotent() {
        let mut once = json!({});
        install_claude_code(&mut once, CMD).unwrap();
        let mut twice = once.clone();
        install_claude_code(&mut twice, CMD).unwrap();
        assert_eq!(once, twice);
    }

    #[test]
    fn uninstall_restores_the_original() {
        let original = json!({
            "hooks": {
                "Stop": [{ "hooks": [{ "type": "command", "command": "say done" }] }],
                "PreToolUse": [{ "matcher": "Bash", "hooks": [{ "type": "command", "command": "lint" }] }]
            },
            "env": { "A": "1" }
        });
        let mut settings = original.clone();
        install_claude_code(&mut settings, CMD).unwrap();
        let removed = uninstall_claude_code(&mut settings).unwrap();
        assert_eq!(removed, CLAUDE_CODE_HOOKS.len());
        assert_eq!(settings, original);
    }

    #[test]
    fn uninstall_removes_empty_hooks_key() {
        let mut settings = json!({ "model": "opus" });
        install_claude_code(&mut settings, CMD).unwrap();
        uninstall_claude_code(&mut settings).unwrap();
        assert_eq!(settings, json!({ "model": "opus" }));
    }

    const CODEX_CMD: &str = "\"/usr/local/bin/agent-graph\" emit --provider codex";

    /// Codex's own test vector (codex-rs/config/src/fingerprint_tests.rs, as
    /// the hook discovery uses it): a command handler with no timeout,
    /// matcher or async.
    #[test]
    fn the_trust_hash_is_codexs() {
        let handler = json!({"type": "command", "command": "python3 /tmp/user.py"});
        assert_eq!(
            codex_trust_hash("session_start", None, &handler),
            "sha256:775a1a39423c99333a34296e0b7c23c35bd26a3f709d4df4fbb3d15304ae8adc"
        );
    }

    #[test]
    fn codex_hooks_go_in_keep_the_rest_and_come_out_again() {
        let original = json!({
            "hooks": {
                "Stop": [{ "hooks": [{ "type": "command", "command": "say done" }] }]
            }
        });
        let mut hooks = original.clone();
        install_codex(&mut hooks, CODEX_CMD).unwrap();
        let stop = hooks["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 2);
        assert_eq!(stop[0]["hooks"][0]["command"], "say done");
        assert_eq!(stop[1]["hooks"][0]["async"], true);
        // Codex runs SessionEnd synchronously, for at most 3 s.
        let end = &hooks["hooks"]["SessionEnd"][0]["hooks"][0];
        assert_eq!(end["timeout"], 3);
        assert!(end.get("async").is_none());
        assert_eq!(our_codex_events(&hooks).len(), CODEX_HOOKS.len());
        let once = hooks.clone();
        install_codex(&mut hooks, CODEX_CMD).unwrap();
        assert_eq!(hooks, once, "installing again changes nothing");
        assert_eq!(uninstall_codex(&mut hooks).unwrap(), CODEX_HOOKS.len());
        assert_eq!(hooks, original);
        // A command it couldn't find again is refused.
        assert!(install_codex(&mut json!({}), "my-wrapper").is_err());
    }

    #[test]
    fn codex_trust_follows_the_handlers() {
        let source = "/home/me/.codex/hooks.json";
        // Your own Stop hook, trusted (and one you turned off), and a comment.
        let before = json!({
            "hooks": {
                "Stop": [{ "hooks": [{ "type": "command", "command": "say done" }] }],
                "PreToolUse": [{ "matcher": "Bash", "hooks": [{ "type": "command", "command": "lint" }] }]
            }
        });
        let config = "# mine\nmodel = \"gpt-5.5\"\n\n[hooks.state.\"/home/me/.codex/hooks.json:stop:0:0\"]\ntrusted_hash = \"sha256:mine\"\n\n[hooks.state.\"/home/me/.codex/hooks.json:pre_tool_use:0:0\"]\nenabled = false\n";
        let mut after = before.clone();
        install_codex(&mut after, CODEX_CMD).unwrap();
        let trusted = codex_trust(config, source, &before, &after).unwrap();
        let doc: toml_edit::DocumentMut = trusted.parse().unwrap();
        let state = doc["hooks"]["state"].as_table_like().unwrap();
        assert!(
            trusted.starts_with("# mine\nmodel = \"gpt-5.5\""),
            "{trusted}"
        );
        // Yours stay as they were; each of ours is trusted, where it is.
        assert_eq!(
            state.get(&format!("{source}:stop:0:0")).unwrap()["trusted_hash"].as_str(),
            Some("sha256:mine")
        );
        assert_eq!(
            state.get(&format!("{source}:pre_tool_use:0:0")).unwrap()["enabled"].as_bool(),
            Some(false)
        );
        for p in placed(&after).iter().filter(|p| p.ours) {
            let entry = state.get(&trust_key(source, p)).expect("trusted");
            assert_eq!(
                entry["trusted_hash"].as_str().unwrap(),
                codex_trust_hash(p.label, p.matcher, p.handler)
            );
        }
        assert_eq!(state.len(), 2 + CODEX_HOOKS.len());

        // Uninstalled, ours go and yours stay.
        let mut removed = after.clone();
        uninstall_codex(&mut removed).unwrap();
        let untrusted = codex_trust(&trusted, source, &after, &removed).unwrap();
        assert_eq!(untrusted, config);

        // Our hooks removed from before yours: your trust moves with them.
        let ours_first = json!({
            "hooks": {
                "Stop": [
                    { "hooks": [{ "type": "command", "command": CODEX_CMD, "async": true }] },
                    { "hooks": [{ "type": "command", "command": "say done" }] }
                ]
            }
        });
        let config = "[hooks.state.\"/s:stop:1:0\"]\ntrusted_hash = \"sha256:mine\"\n";
        let mut gone = ours_first.clone();
        uninstall_codex(&mut gone).unwrap();
        let moved = codex_trust(config, "/s", &ours_first, &gone).unwrap();
        assert!(moved.contains("\"/s:stop:0:0\""), "{moved}");
        assert!(!moved.contains("stop:1:0"), "{moved}");
        // Nothing to trust, in an empty file, leaves it empty.
        assert_eq!(codex_trust("", "/s", &json!({}), &json!({})).unwrap(), "");
    }

    #[test]
    fn rejects_malformed_hooks() {
        let mut settings = json!({ "hooks": [] });
        assert!(install_claude_code(&mut settings, CMD).is_err());
    }
}
