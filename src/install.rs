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
    let found = crate::paths::find_program("agent-graph")?;
    let this = std::env::current_exe().ok()?;
    Some(
        match (std::fs::canonicalize(found), std::fs::canonicalize(this)) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        },
    )
}

/// The hook command pointing at this executable. Forward slashes and quotes
/// keep it working in the shells Claude Code uses on every platform.
pub fn default_command(provider: &str) -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| format!("can't find this executable: {e}"))?;
    let exe = exe.to_string_lossy().replace('\\', "/");
    Ok(format!("\"{exe}\" emit --provider {provider}"))
}

/// Adds our hooks to Claude Code `settings`, replacing any earlier copy.
pub fn install_claude_code(settings: &mut Value, command: &str) -> Result<(), String> {
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
        .is_some_and(|c| c.contains(CLAUDE_CODE_MARKER))
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
