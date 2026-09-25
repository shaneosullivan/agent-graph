//! Spotting shell commands that start another agent session, like
//! `codex exec "…"` or `claude -p "…"` run from an agent's shell tool.
//!
//! This reads commands the way a shell roughly would: it splits them into
//! simple commands at `;`, `&&`, `||`, `|`, `&`, newlines, `(` and command
//! substitutions, outside quotes, then looks at the program each one runs,
//! past variable assignments and wrappers like `env`, `nohup` or `timeout`.
//! It's a heuristic: a script that starts an agent inside itself isn't seen,
//! though the agent still links itself to the session (see `link`).

/// Agent CLIs that start a session. `AGENT_GRAPH_AGENT_COMMANDS` (a comma
/// separated list) adds more.
pub const AGENT_COMMANDS: &[&str] = &[
    "claude",
    "codex",
    "gemini",
    "cursor-agent",
    "copilot",
    "opencode",
    "amp",
    "aider",
    "goose",
    "qwen",
];

/// Subcommands that manage an agent rather than start a session.
const NOT_SESSIONS: &[(&str, &[&str])] = &[
    (
        "claude",
        &[
            "mcp",
            "config",
            "update",
            "upgrade",
            "doctor",
            "install",
            "setup-token",
            "plugin",
            "migrate-installer",
            "auth",
        ],
    ),
    (
        "codex",
        &[
            "login",
            "logout",
            "mcp",
            "mcp-server",
            "completion",
            "apply",
            "debug",
            "sandbox",
            "features",
            "help",
        ],
    ),
    ("gemini", &["mcp", "extensions"]),
];

/// Shell keywords that can come before a command.
const KEYWORDS: &[&str] = &["do", "then", "else", "if", "elif", "while", "until", "!"];

/// Programs that run the rest of their arguments as a command.
const WRAPPERS: &[&str] = &[
    "env",
    "nohup",
    "time",
    "nice",
    "exec",
    "command",
    "sudo",
    "caffeinate",
    "npx",
    "bunx",
    "stdbuf",
    "timeout",
];

/// A command that starts an agent session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    /// The agent's program, e.g. `codex`. For `agent-graph run -- X`, X's.
    pub program: String,
    /// It's put in the background with `&`, so the shell doesn't wait.
    pub background: bool,
}

/// The first agent session `command` starts, if any. `extra` adds programs
/// to `AGENT_COMMANDS`.
pub fn agent_launch(command: &str, extra: &[String]) -> Option<Launch> {
    let commands = split(command);
    commands.iter().find_map(|(words, background)| {
        let program = session_program(words, extra)?;
        Some(Launch {
            program,
            background: *background,
        })
    })
}

/// The extra programs in `AGENT_GRAPH_AGENT_COMMANDS`.
pub fn extra_commands(value: Option<&str>) -> Vec<String> {
    value
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

/// The program a simple command runs, if it starts an agent session.
fn session_program(words: &[String], extra: &[String]) -> Option<String> {
    let mut rest = skip_prefixes(words);
    let program = program_name(rest.first()?);
    // `agent-graph run [--name N] -- X …` starts X, linked to us.
    if program == "agent-graph" {
        if rest.get(1).map(String::as_str) != Some("run") {
            return None;
        }
        let dashes = rest.iter().position(|w| w == "--")?;
        rest = skip_prefixes(&rest[dashes + 1..]);
        let wrapped = program_name(rest.first()?);
        return (!wrapped.is_empty()).then_some(wrapped);
    }
    let known = AGENT_COMMANDS.contains(&program.as_str()) || extra.contains(&program);
    if !known {
        return None;
    }
    let args = &rest[1..];
    let asks_about_itself = args
        .iter()
        .any(|a| matches!(a.as_str(), "--version" | "-v" | "-V" | "--help" | "-h"));
    let first = args.iter().find(|a| !a.starts_with('-'));
    let manages = NOT_SESSIONS
        .iter()
        .find(|(p, _)| *p == program)
        .is_some_and(|(_, subs)| first.is_some_and(|f| subs.contains(&f.as_str())));
    (!asks_about_itself && !manages).then_some(program)
}

/// Skips `NAME=value` assignments and wrapper programs (with their options,
/// and `timeout`'s duration) at the start of a simple command.
fn skip_prefixes(words: &[String]) -> &[String] {
    let mut i = 0;
    while let Some(word) = words.get(i) {
        if is_assignment(word) || KEYWORDS.contains(&word.as_str()) {
            i += 1;
            continue;
        }
        let name = program_name(word);
        if !WRAPPERS.contains(&name.as_str()) {
            break;
        }
        i += 1;
        while words.get(i).is_some_and(|w| w.starts_with('-')) {
            i += 1;
        }
        if name == "timeout" && words.get(i).is_some() {
            i += 1;
        }
    }
    &words[i.min(words.len())..]
}

fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty()
            && name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// A program's name without its folder or Windows extension.
fn program_name(word: &str) -> String {
    let base = word.rsplit(['/', '\\']).next().unwrap_or(word);
    let lower = base.to_ascii_lowercase();
    [".exe", ".cmd", ".bat"]
        .iter()
        .find_map(|ext| lower.strip_suffix(ext))
        .unwrap_or(&lower)
        .to_string()
}

/// Splits `command` into simple commands (as words, with quotes removed),
/// each with whether it's put in the background.
fn split(command: &str) -> Vec<(Vec<String>, bool)> {
    let mut out: Vec<(Vec<String>, bool)> = Vec::new();
    let mut words: Vec<String> = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut chars = command.chars().peekable();
    let mut single = false;
    let mut double = false;

    let end_word = |words: &mut Vec<String>, word: &mut String, in_word: &mut bool| {
        if *in_word {
            words.push(std::mem::take(word));
            *in_word = false;
        }
    };
    let end_command = |out: &mut Vec<(Vec<String>, bool)>, words: &mut Vec<String>, bg: bool| {
        if !words.is_empty() {
            out.push((std::mem::take(words), bg));
        }
    };

    while let Some(c) = chars.next() {
        if single {
            if c == '\'' {
                single = false;
            } else {
                word.push(c);
            }
            continue;
        }
        match c {
            '\\' => {
                if let Some(next) = chars.next() {
                    word.push(next);
                    in_word = true;
                }
            }
            '\'' if !double => {
                single = true;
                in_word = true;
            }
            '"' => {
                double = !double;
                in_word = true;
            }
            // A command substitution runs a command of its own, even inside
            // double quotes: `out="$(codex exec …)"`.
            '$' if chars.peek() == Some(&'(') => {
                chars.next();
                end_word(&mut words, &mut word, &mut in_word);
                end_command(&mut out, &mut words, false);
                double = false;
            }
            '`' => {
                end_word(&mut words, &mut word, &mut in_word);
                end_command(&mut out, &mut words, false);
                double = false;
            }
            _ if double => {
                word.push(c);
                in_word = true;
            }
            ' ' | '\t' => end_word(&mut words, &mut word, &mut in_word),
            ';' | '\n' | '(' | ')' | '{' | '}' => {
                end_word(&mut words, &mut word, &mut in_word);
                end_command(&mut out, &mut words, false);
            }
            '|' => {
                chars.next_if_eq(&'|');
                end_word(&mut words, &mut word, &mut in_word);
                end_command(&mut out, &mut words, false);
            }
            // `2>&1` and `&>` redirect: the `&` is part of the word.
            '&' if word.ends_with('>') || chars.peek() == Some(&'>') => {
                word.push(c);
                in_word = true;
            }
            '&' => {
                end_word(&mut words, &mut word, &mut in_word);
                // `&&` runs the next command after this one; a lone `&` puts
                // this one in the background.
                let background = chars.next_if_eq(&'&').is_none();
                end_command(&mut out, &mut words, background);
            }
            _ => {
                word.push(c);
                in_word = true;
            }
        }
    }
    end_word(&mut words, &mut word, &mut in_word);
    end_command(&mut out, &mut words, false);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn launch(command: &str) -> Option<(String, bool)> {
        agent_launch(command, &[]).map(|l| (l.program, l.background))
    }

    fn fg(program: &str) -> Option<(String, bool)> {
        Some((program.to_string(), false))
    }

    #[test]
    fn spots_agents_started_in_the_foreground() {
        assert_eq!(launch("codex exec 'review the diff'"), fg("codex"));
        assert_eq!(
            launch(r#"claude -p "fix the tests" --output-format json"#),
            fg("claude")
        );
        assert_eq!(launch("cd sub && gemini -p hi"), fg("gemini"));
        assert_eq!(launch("/usr/local/bin/claude -p hi"), fg("claude"));
        assert_eq!(launch(r"'C:\tools\Claude.exe' -p hi"), fg("claude"));
        assert_eq!(
            launch("FOO=1 BAR='a b' timeout 600 nohup codex exec x"),
            fg("codex")
        );
        assert_eq!(launch("env -i PATH=/bin npx -y codex exec x"), fg("codex"));
        assert_eq!(
            launch("git diff | claude -p 'review this' > out.txt 2>&1"),
            fg("claude")
        );
        assert_eq!(launch(r#"out="$(codex exec 'summarise')""#), fg("codex"));
        assert_eq!(launch("result=`claude -p hi`"), fg("claude"));
        assert_eq!(launch("(cd app; aider --yes)"), fg("aider"));
        assert_eq!(
            launch("for f in *.md; do claude -p \"$f\"; done"),
            fg("claude")
        );
        assert_eq!(launch("if true; then codex exec x; fi"), fg("codex"));
    }

    #[test]
    fn spots_background_launches() {
        assert_eq!(
            launch("codex exec 'long job' &"),
            Some(("codex".into(), true))
        );
        assert_eq!(
            launch("claude -p a > a.log 2>&1 & claude -p b &"),
            Some(("claude".into(), true))
        );
        assert_eq!(launch("claude -p a &> log.txt"), fg("claude"));
    }

    #[test]
    fn follows_agent_graph_run() {
        assert_eq!(launch("agent-graph run -- aider --yes"), fg("aider"));
        assert_eq!(
            launch("~/bin/agent-graph run --name worker -- env X=1 ./worker.sh &"),
            Some(("worker.sh".into(), true))
        );
        assert_eq!(launch("agent-graph tree"), None);
        assert_eq!(launch("agent-graph run"), None);
    }

    #[test]
    fn ignores_everything_else() {
        for command in [
            "",
            "ls -la",
            "cargo test",
            "echo 'run claude -p later; codex exec x'",
            r#"echo "then claude -p hi""#,
            "grep -r codex src/",
            "git commit -m 'claude: fix'",
            "claude --version",
            "claude -h",
            "claude mcp list",
            "codex login",
            "codex --help",
            "gemini extensions list",
            "which claude",
            "cat claude.log",
        ] {
            assert_eq!(launch(command), None, "{command:?}");
        }
    }

    #[test]
    fn extra_commands_come_from_the_environment() {
        let extra = extra_commands(Some(" my-agent , other,, "));
        assert_eq!(extra, ["my-agent", "other"]);
        assert_eq!(
            agent_launch("my-agent go", &extra)
                .map(|l| l.program)
                .as_deref(),
            Some("my-agent")
        );
        assert_eq!(extra_commands(None), Vec::<String>::new());
    }
}
