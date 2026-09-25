//! The `/agent-graph` command that `agent-graph install` adds to each coding
//! agent, so you can ask any of them to show you the graph.
//!
//! Most clients now use "skills" (a folder with a SKILL.md): Claude Code,
//! Codex and Cursor. Gemini CLI has TOML custom commands. All of them get the
//! same instructions; only the wrapping and how arguments arrive differ.
//!
//! Each file carries MARKER, so reinstalling updates only a file Agent Graph
//! wrote, and uninstalling never removes one the user wrote themselves.

use std::path::{Path, PathBuf};

use crate::install::Scope;

pub const NAME: &str = "agent-graph";
pub const MARKER: &str = "Installed by agent-graph";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Client {
    ClaudeCode,
    Codex,
    Gemini,
    Cursor,
}

pub fn client_name(client: Client) -> &'static str {
    match client {
        Client::ClaudeCode => "Claude Code",
        Client::Codex => "Codex",
        Client::Gemini => "Gemini CLI",
        Client::Cursor => "Cursor",
    }
}

pub struct CommandFile {
    pub path: PathBuf,
    pub contents: String,
    /// What the user types to run it.
    pub invoke: &'static str,
}

/// Where the command goes for `client` and `scope`, and what it says.
///
/// There's no uncommitted, per-project place for commands, so the local scope
/// uses the project's folder, like the project scope.
pub fn command_file(client: Client, scope: Scope, project: &Path) -> Result<CommandFile, String> {
    let home = || crate::paths::user_home().ok_or("can't find your home directory".to_string());
    let skill = |base: PathBuf| base.join("skills").join(NAME).join("SKILL.md");
    let (path, contents, invoke) = match client {
        Client::ClaudeCode => {
            let base = match scope {
                Scope::User => {
                    match std::env::var_os("CLAUDE_CONFIG_DIR").filter(|v| !v.is_empty()) {
                        Some(dir) => PathBuf::from(dir),
                        None => home()?.join(".claude"),
                    }
                }
                Scope::Project | Scope::Local => project.join(".claude"),
            };
            (skill(base), claude_skill(), "/agent-graph")
        }
        // Codex reads skills from ~/.agents/skills (and .agents/skills in a
        // project); ~/.codex/skills and custom prompts are deprecated.
        Client::Codex => {
            let base = match scope {
                Scope::User => home()?.join(".agents"),
                Scope::Project | Scope::Local => project.join(".agents"),
            };
            (skill(base), generic_skill(), "$agent-graph")
        }
        Client::Cursor => {
            let base = match scope {
                Scope::User => home()?.join(".cursor"),
                Scope::Project | Scope::Local => project.join(".cursor"),
            };
            (skill(base), generic_skill(), "/agent-graph")
        }
        Client::Gemini => {
            let base = match scope {
                Scope::User => {
                    match std::env::var_os("GEMINI_CLI_HOME").filter(|v| !v.is_empty()) {
                        Some(dir) => PathBuf::from(dir).join(".gemini"),
                        None => home()?.join(".gemini"),
                    }
                }
                Scope::Project | Scope::Local => project.join(".gemini"),
            };
            (
                base.join("commands").join(format!("{NAME}.toml")),
                gemini_command(),
                "/agent-graph",
            )
        }
    };
    Ok(CommandFile {
        path,
        contents,
        invoke,
    })
}

/// Whether a command file is one Agent Graph wrote.
pub fn is_ours(text: &str) -> bool {
    text.contains(MARKER)
}

/// A skill's own folder (…/skills/agent-graph), which uninstalling removes
/// once it's empty.
pub fn is_own_folder(dir: &Path) -> bool {
    dir.file_name().is_some_and(|n| n == NAME)
        && dir
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|n| n == "skills")
}

const DESCRIPTION: &str = "Show the user their Agent Graph: a picture and summary of the AI coding \
     sessions and agents on this machine, what each is doing, and what needs \
     them. Can also share it as a live link.";

/// The description as a YAML double-quoted string (it contains ": ", which a
/// plain YAML value can't).
fn yaml_description() -> String {
    format!(
        "\"{}\"",
        DESCRIPTION.replace('\\', "\\\\").replace('"', "\\\"")
    )
}

fn marker_comment() -> String {
    format!(
        "<!-- {MARKER}. `agent-graph uninstall` removes this file; reinstalling replaces it. -->"
    )
}

/// Claude Code: `$ARGUMENTS` carries what was typed after the command, and
/// the exact local, read-only commands the instructions use are pre-approved.
/// No wildcards: `snapshot --out` writes a file, so any other form (and
/// sharing, which sends data to a website) keeps its permission prompt.
fn claude_skill() -> String {
    format!(
        "---\nname: {NAME}\ndescription: {}\nargument-hint: \"[all | share | <session id>]\"\n\
         allowed-tools: Bash(agent-graph tree) Bash(agent-graph tree --all) \
         Bash(agent-graph snapshot --session current) Bash(agent-graph snapshot --all)\n---\n\n{}\n\n{}",
        yaml_description(),
        marker_comment(),
        instructions("The user asked for: \"$ARGUMENTS\" (it may be empty).")
    )
}

/// Codex and Cursor skills: how typed arguments reach a skill isn't settled
/// across clients, so the instructions ask the model to read them from the
/// conversation.
fn generic_skill() -> String {
    format!(
        "---\nname: {NAME}\ndescription: {}\n---\n\n{}\n\n{}",
        yaml_description(),
        marker_comment(),
        instructions(
            "Work out what the user asked for from their message: any words after the command \
             name, such as `all`, `share` or a session id (it may be nothing)."
        )
    )
}

/// Gemini CLI: `{{args}}` carries what was typed after the command.
fn gemini_command() -> String {
    let body = instructions("The user asked for: \"{{args}}\" (it may be empty).");
    format!(
        "# {MARKER}. `agent-graph uninstall` removes this file; reinstalling replaces it.\n\
         description = \"{}\"\nprompt = '''\n{body}'''\n",
        DESCRIPTION.replace('"', "\\\"")
    )
}

/// What every client is asked to do.
fn instructions(request: &str) -> String {
    format!(
        r#"Show the user their Agent Graph: the AI coding sessions and agents on this machine, what each is doing, and what needs the user.

{request}

- Nothing, or a session id: this session (or the one given).
- `all`: every session active in the last day.
- `share`: a live link they can open anywhere, such as on their phone.

## A picture and summary (the default, and `all`)

1. Run `agent-graph tree` (add `--all` for `all`) and read the result.
2. Run `agent-graph snapshot --session current` (use `--all` instead for `all`, or `--session <id>` for a given session). It prints only the path of a PNG sized for a phone.
3. If you have a tool that sends files or images to the user, send them the PNG with it. Otherwise give them its path.
4. Then summarise in a few short lines, most urgent first:
   - anything that **needs the user** (a permission prompt, a question, a plan to approve), and in which session or agent;
   - anything that **looks stuck** (`stale?`) or is **deadlocked**;
   - what's working now, and how many tasks are left.
   Don't repeat the whole tree.

## A live link (`share`)

Only when the user asks to share it or for a link:

1. Run `agent-graph watch-remote --session current` as a background command. It keeps running, streaming new events to the Agent Graph site until it's stopped. Leave out `--session current` only if the user asks to share every session.
2. Its first line of output is the link. Give it to the user, and tell them it stays live until the command is stopped.
3. Anyone with the link can view it, unless the user saved a default password with `--save-default-password`. Don't choose a password for them; pass `--password=<password>` only if they give you one.

If the `agent-graph` command isn't found, tell the user Agent Graph isn't installed, and stop.
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> PathBuf {
        crate::paths::user_home().unwrap()
    }

    #[test]
    fn each_client_gets_its_own_file() {
        let project = Path::new("/work/app");
        let cases = [
            (
                Client::ClaudeCode,
                Scope::Project,
                project.join(".claude/skills/agent-graph/SKILL.md"),
                "/agent-graph",
            ),
            (
                Client::Codex,
                Scope::User,
                home().join(".agents/skills/agent-graph/SKILL.md"),
                "$agent-graph",
            ),
            (
                Client::Cursor,
                Scope::Project,
                project.join(".cursor/skills/agent-graph/SKILL.md"),
                "/agent-graph",
            ),
            (
                Client::Gemini,
                Scope::Project,
                project.join(".gemini/commands/agent-graph.toml"),
                "/agent-graph",
            ),
        ];
        for (client, scope, path, invoke) in cases {
            let file = command_file(client, scope, project).unwrap();
            assert_eq!(file.path, path, "{client:?}");
            assert_eq!(file.invoke, invoke);
            assert!(is_ours(&file.contents));
        }
    }

    #[test]
    fn skills_have_valid_frontmatter() {
        for client in [Client::ClaudeCode, Client::Codex, Client::Cursor] {
            let text = command_file(client, Scope::Project, Path::new("/p"))
                .unwrap()
                .contents;
            let front = text
                .strip_prefix("---\n")
                .unwrap()
                .split("\n---\n")
                .next()
                .unwrap();
            assert!(front.contains("name: agent-graph\n"), "{client:?}");
            // Quoted: the description contains ": ", which a plain YAML value can't.
            assert!(
                front
                    .lines()
                    .any(|l| l.starts_with("description: \"") && l.ends_with('"')),
                "{client:?}"
            );
            // One line per key: a description that wrapped would break YAML.
            assert!(
                front.lines().all(|l| l.contains(": ")),
                "{client:?}: {front}"
            );
        }
    }

    #[test]
    fn claude_pre_approves_only_local_commands() {
        let text = claude_skill();
        let tools = text
            .lines()
            .find(|l| l.starts_with("allowed-tools:"))
            .unwrap();
        // Only the exact commands the instructions use. A wildcard would also
        // approve `snapshot --out <any file>` (R2).
        let allowed: Vec<&str> = tools
            .trim_start_matches("allowed-tools:")
            .split(") ")
            .map(|t| t.trim().trim_end_matches(')'))
            .collect();
        assert_eq!(
            allowed,
            [
                "Bash(agent-graph tree",
                "Bash(agent-graph tree --all",
                "Bash(agent-graph snapshot --session current",
                "Bash(agent-graph snapshot --all",
            ]
        );
        assert!(!tools.contains('*'), "no wildcards");
        assert!(
            !tools.contains("watch-remote"),
            "sharing keeps its permission prompt"
        );
        assert!(text.contains("$ARGUMENTS"));
    }

    #[test]
    fn gemini_command_is_toml_with_args() {
        let text = gemini_command();
        assert!(text.contains("{{args}}"));
        assert!(text.contains("\nprompt = '''\n"));
        // A literal ''' inside the prompt would end the TOML string early.
        let prompt = text.split("prompt = '''").nth(1).unwrap();
        assert_eq!(prompt.matches("'''").count(), 1);
    }

    #[test]
    fn own_folder_is_the_skill_folder() {
        assert!(is_own_folder(Path::new("/h/.claude/skills/agent-graph")));
        assert!(!is_own_folder(Path::new("/h/.gemini/commands")));
        assert!(!is_own_folder(Path::new("/h/.claude/skills")));
    }
}
