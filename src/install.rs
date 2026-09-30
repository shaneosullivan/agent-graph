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
            handlers.retain(|h| !is_ours(h));
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
                        .is_some_and(|hs| hs.iter().any(is_ours))
                })
            })
        })
        .map(|(event, _)| event.clone())
        .collect()
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

    #[test]
    fn rejects_malformed_hooks() {
        let mut settings = json!({ "hooks": [] });
        assert!(install_claude_code(&mut settings, CMD).is_err());
    }
}
