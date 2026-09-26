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
    // AWS Copilot shares the name; GitHub's Copilot CLI takes a prompt.
    (
        "copilot",
        &[
            "init",
            "app",
            "env",
            "svc",
            "job",
            "pipeline",
            "deploy",
            "storage",
            "task",
            "secret",
            "docs",
            "version",
            "completion",
            "help",
            "run",
            // GitHub's Copilot CLI's own management commands.
            "login",
            "logout",
            "mcp",
            "plugin",
            "update",
            "instruction",
            "lsp",
            "skill",
        ],
    ),
];

/// Agents whose subcommand, if any, comes first (so a later word is an
/// option's value: `copilot --add-dir docs -p …`).
const SUBCOMMAND_FIRST: &[&str] = &["copilot"];

/// Agents that start a session only with one of these subcommands (the
/// database migration tool `goose` shares Block's agent's name).
const ONLY_SESSIONS: &[(&str, &[&str])] = &[("goose", &["session", "run", "review"])];

/// Subcommands that manage sessions rather than start one: the program,
/// its subcommand, and what comes right after that (`goose session list`).
const MANAGES_SESSIONS: &[(&str, &str, &[&str])] = &[(
    "goose",
    "session",
    &["list", "remove", "export", "diagnostics"],
)];

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
    let first_word = rest.first()?;
    // `agent-graph run [--name N] -- X …` starts X, linked to us.
    if named(first_word, &["agent-graph"]).is_some() {
        if rest.get(1).map(String::as_str) != Some("run") {
            return None;
        }
        let dashes = rest.iter().position(|w| w == "--")?;
        rest = skip_prefixes(&rest[dashes + 1..]);
        let wrapped = program_name(rest.first()?);
        return (!wrapped.is_empty()).then_some(wrapped);
    }
    let program = named(first_word, AGENT_COMMANDS)
        .map(str::to_string)
        .or_else(|| named(first_word, extra).map(str::to_string))?;
    let args = &rest[1..];
    let asks_about_itself = args
        .iter()
        .any(|a| matches!(a.as_str(), "--version" | "-v" | "-V" | "--help" | "-h"));
    let first = if SUBCOMMAND_FIRST.contains(&program.as_str()) {
        args.first().filter(|a| !a.starts_with('-'))
    } else {
        args.iter().find(|a| !a.starts_with('-'))
    }
    .map(String::as_str);
    let manages = NOT_SESSIONS
        .iter()
        .find(|(p, _)| *p == program)
        .is_some_and(|(_, subs)| first.is_some_and(|f| subs.contains(&f)))
        || MANAGES_SESSIONS.iter().any(|(p, sub, subs)| {
            *p == program
                && args.first().is_some_and(|a| a == sub)
                && args.get(1).is_some_and(|a| subs.contains(&a.as_str()))
        });
    let not_a_session = ONLY_SESSIONS
        .iter()
        .find(|(p, _)| *p == program)
        .is_some_and(|(_, subs)| !first.is_some_and(|f| subs.contains(&f)));
    (!asks_about_itself && !manages && !not_a_session).then_some(program)
}

/// Which of `names` `word` runs, if any: by its exact name, or for a Windows
/// program named with its extension, in any case (`Claude.exe`).
fn named<'a, S: AsRef<str>>(word: &str, names: &'a [S]) -> Option<&'a str> {
    let name = program_name(word);
    let windows = name.len() < word.rsplit(['/', '\\']).next().unwrap_or(word).len();
    names.iter().map(AsRef::as_ref).find(|n| {
        if windows {
            n.eq_ignore_ascii_case(&name)
        } else {
            *n == name
        }
    })
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
        let Some(name) = named(word, WRAPPERS) else {
            break;
        };
        i += 1;
        let options = i;
        while words.get(i).is_some_and(|w| w.starts_with('-')) {
            i += 1;
        }
        // `command -v X` (or `-V`, `-pv`, …) looks X up: it runs nothing.
        let lookup = |o: &String| !o.starts_with("--") && o[1..].contains(['v', 'V']);
        if name == "command" && words[options..i].iter().any(lookup) {
            return &words[words.len()..];
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

/// A program's name without its folder or Windows extension, in its own
/// case (prose like "Claude & Codex …" isn't a command; see `named`).
/// `agent-graph run` names its sessions with it too, so the two agree.
pub fn program_name(word: &str) -> String {
    let base = word.rsplit(['/', '\\']).next().unwrap_or(word);
    let lower = base.to_ascii_lowercase();
    [".exe", ".cmd", ".bat"]
        .iter()
        .find(|ext| lower.ends_with(*ext))
        .map_or(base, |ext| &base[..base.len() - ext.len()])
        .to_string()
}

/// How far to look for the end of what may be arithmetic: no arithmetic is
/// longer, and it keeps a long run of `(` from taking quadratic time.
const ARITHMETIC_MAX: usize = 1024;

/// Whether what follows a `((` (`chars`) is arithmetic, as the shell reads
/// it: the parentheses close together as `))`, soon, and it holds no command
/// substitution. Otherwise it's a command substitution or subshells, of
/// commands (`$((cd a && codex exec x) 2>&1)`).
fn closes_as_arithmetic(chars: std::iter::Peekable<std::str::Chars>) -> bool {
    let mut chars = chars.take(ARITHMETIC_MAX).peekable();
    let mut depth = 2;
    while let Some(c) = chars.next() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 1 {
                    return chars.peek() == Some(&')');
                }
            }
            '$' if chars.peek() == Some(&'(') => return false,
            '`' => return false,
            _ => {}
        }
    }
    false
}

/// Skips an arithmetic expression, `((` already read, up to its `))`,
/// adding it to `word`: it runs no commands, and its `<<` is a shift.
fn skip_arithmetic(chars: &mut std::iter::Peekable<std::str::Chars>, word: &mut String) {
    word.push_str("((");
    let mut depth = 2;
    for c in chars.by_ref() {
        word.push(c);
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ => {}
        }
        if depth == 0 {
            break;
        }
    }
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
    // Heredocs whose bodies start after this line: (delimiter, `<<-`).
    let mut heredocs: Vec<(String, bool)> = Vec::new();

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
            // `$[…]`: arithmetic, the old way.
            '$' if chars.peek() == Some(&'[') => {
                word.push(c);
                for c in chars.by_ref() {
                    word.push(c);
                    if c == ']' {
                        break;
                    }
                }
                in_word = true;
            }
            // A command substitution runs a command of its own, even inside
            // double quotes: `out="$(codex exec …)"`.
            '$' if chars.peek() == Some(&'(') => {
                chars.next();
                // `$((…))` is arithmetic, not a command.
                if chars.peek() == Some(&'(') && {
                    let mut ahead = chars.clone();
                    ahead.next();
                    closes_as_arithmetic(ahead)
                } {
                    chars.next();
                    word.push('$');
                    skip_arithmetic(&mut chars, &mut word);
                    in_word = true;
                    continue;
                }
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
            // `<<-? WORD`: a heredoc, whose body (the lines after this one,
            // up to WORD) is text, not commands. (`<<<` is a here-string:
            // no WORD follows, so no body.)
            '<' if chars.peek() == Some(&'<') => {
                chars.next();
                end_word(&mut words, &mut word, &mut in_word);
                let tabs = chars.next_if_eq(&'-').is_some();
                while chars.next_if(|c| *c == ' ' || *c == '\t').is_some() {}
                // WORD, unquoted: quotes can hold spaces (`<<"END OF"`), or
                // nothing (`<<''`, which a blank line ends).
                let mut delimiter = String::new();
                let mut quoted = false;
                loop {
                    match chars.peek() {
                        Some(&q @ ('\'' | '"')) => {
                            chars.next();
                            quoted = true;
                            while let Some(c) = chars.next_if(|c| *c != q) {
                                delimiter.push(c);
                            }
                            chars.next();
                        }
                        Some('\\') => {
                            chars.next();
                            quoted = true;
                            delimiter.extend(chars.next());
                        }
                        Some(c) if !c.is_whitespace() && !";|&()<>".contains(*c) => {
                            delimiter.extend(chars.next());
                        }
                        _ => break,
                    }
                }
                if !delimiter.is_empty() || quoted {
                    heredocs.push((delimiter, tabs));
                }
            }
            ' ' | '\t' | '\r' => end_word(&mut words, &mut word, &mut in_word),
            '\n' => {
                end_word(&mut words, &mut word, &mut in_word);
                end_command(&mut out, &mut words, false);
                for (delimiter, tabs) in heredocs.drain(..) {
                    loop {
                        let line: String = chars.by_ref().take_while(|c| *c != '\n').collect();
                        let line = line.trim_end_matches('\r');
                        let line = if tabs {
                            line.trim_start_matches('\t')
                        } else {
                            line
                        };
                        if line == delimiter || chars.peek().is_none() {
                            break;
                        }
                    }
                }
            }
            // `((…))` starting a command (after any keywords: `if ((…))`,
            // `for ((…))`) is arithmetic too.
            '(' if chars.peek() == Some(&'(')
                && !in_word
                && words
                    .iter()
                    .all(|w| KEYWORDS.contains(&w.as_str()) || w == "for")
                && {
                    let mut ahead = chars.clone();
                    ahead.next();
                    closes_as_arithmetic(ahead)
                } =>
            {
                chars.next();
                skip_arithmetic(&mut chars, &mut word);
                in_word = true;
            }
            ';' | '(' | ')' | '{' | '}' => {
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

    /// R25: a heredoc's body is text, not commands: every commit message
    /// written with one would otherwise be read as a script.
    #[test]
    fn heredoc_bodies_arent_commands() {
        let commit = "git commit -m \"$(cat <<'EOF'\nFix the parser\n\nclaude & codex both work now\nEOF\n)\"";
        assert_eq!(launch(commit), None);
        assert_eq!(launch("cat > notes.md <<EOF\nclaude -p hi\nEOF"), None);
        assert_eq!(launch("cat <<\"END\" | wc\ncodex exec x\nEND"), None);
        // Where the body ends: with `<<-`, at a tab-indented WORD; with
        // Windows line endings, at WORD before the CR.
        assert_eq!(
            launch("cat <<-EOF\n\tclaude -p hi\n\tEOF\ncodex exec x"),
            fg("codex"),
            "<<- strips tabs"
        );
        assert_eq!(
            launch("cat <<EOF\r\nclaude -p hi\r\nEOF\r\ncodex exec x\r\n"),
            fg("codex"),
            "CRLF"
        );
        // What follows the body is a command again...
        assert_eq!(launch("cat <<EOF > x\nhi\nEOF\nclaude -p hi"), fg("claude"));
        // ...and a here-string isn't a heredoc.
        assert_eq!(launch("wc <<< hi; codex exec x"), fg("codex"));
    }

    /// R25: program names are matched exactly, so prose that happens to
    /// start with one ("Claude & Codex …") isn't a launch; a Windows program
    /// with its extension is matched in any case.
    #[test]
    fn names_are_matched_exactly() {
        assert_eq!(launch("Claude -p hi"), None);
        assert_eq!(launch("echo ok; Codex exec x"), None);
        assert_eq!(launch(r"'C:\tools\CLAUDE.EXE' -p hi"), fg("claude"));
    }

    /// R25: `command -v claude` looks claude up; it doesn't run it.
    #[test]
    fn command_lookups_arent_launches() {
        assert_eq!(launch("command -v claude"), None);
        assert_eq!(launch("command -V codex && echo yes"), None);
        assert_eq!(launch("command -pv claude"), None);
        assert_eq!(launch("command -vp claude"), None);
        assert_eq!(launch("command claude -p hi"), fg("claude"));
        assert_eq!(launch("command -p claude -p hi"), fg("claude"));
    }

    /// R25: other tools called `goose` (the database migration tool) and
    /// `copilot` (AWS Copilot) aren't agents.
    #[test]
    fn other_tools_with_agents_names_arent_launches() {
        for command in [
            "goose up",
            "goose -dir db postgres \"$DSN\" status",
            "goose create add_users sql",
            "copilot app init",
            "copilot deploy --name api",
            "copilot svc logs",
        ] {
            assert_eq!(launch(command), None, "{command}");
        }
        assert_eq!(launch("goose session"), fg("goose"));
        assert_eq!(launch("goose run -t 'fix it'"), fg("goose"));
        assert_eq!(launch("copilot -p 'fix it'"), fg("copilot"));
    }

    /// R25: what a heredoc is and isn't, exactly.
    #[test]
    fn heredocs_are_read_as_the_shell_reads_them() {
        // A here-string has no body.
        assert_eq!(launch("wc <<< hi\ncodex exec x"), fg("codex"));
        // Quoted, `<<` is text.
        assert_eq!(launch("echo \"a <<EOF\"\nclaude -p hi"), fg("claude"));
        // Several on a line: their bodies follow in turn (the first's can
        // hold the second's WORD).
        assert_eq!(
            launch("cat <<A <<B\nB\nA\nclaude -p hi\nB\ncodex exec x"),
            fg("codex")
        );
        // The rest of the heredoc's own line is still commands.
        assert_eq!(
            launch("cat <<'EOF' | claude -p -\nprompt\nEOF"),
            fg("claude")
        );
        // WORD can be quoted with a space in it, or be nothing at all
        // (then a blank line ends it).
        assert_eq!(
            launch("cat <<\"END OF\"\nclaude -p hi\nEND OF\ncodex exec x"),
            fg("codex")
        );
        assert_eq!(
            launch("cat <<''\nclaude -p hi\n\ncodex exec x"),
            fg("codex")
        );
        // Only WORD itself ends it: not indented by a tab (without `-`),
        // nor by spaces (even with it).
        assert_eq!(launch("cat <<EOF\n\tEOF\nclaude -p hi"), None);
        assert_eq!(launch("cat <<-EOF\n  EOF\nclaude -p hi"), None);
    }

    /// R25: arithmetic's `<<` is a shift, not a heredoc.
    #[test]
    fn arithmetic_isnt_a_heredoc() {
        assert_eq!(launch("N=$((1<<4))\nclaude -p \"shard $N\""), fg("claude"));
        assert_eq!(launch("(( x <<= 1 ))\nclaude -p hi"), fg("claude"));
        assert_eq!(
            launch("if (( 1 << 2 > 3 )); then echo y; fi\nclaude -p hi"),
            fg("claude")
        );
        assert_eq!(
            launch("for ((i=1; i<64; i<<=1)); do :; done\nclaude -p hi"),
            fg("claude")
        );
        // A command substitution in it runs, and so does what follows one
        // that never closes.
        assert_eq!(launch("n=$(( `claude -p count` + 1 ))"), fg("claude"));
        assert_eq!(launch("echo $((1+2)\nclaude -p hi"), fg("claude"));
        // Nor arithmetic the old way.
        assert_eq!(launch("echo $[1<<2]\nclaude -p hi"), fg("claude"));
        // Nor something far too long for it (a long run of `(`, say, which
        // would otherwise take time in proportion to its square).
        let long = format!("(({}))", " ".repeat(ARITHMETIC_MAX));
        assert!(!closes_as_arithmetic(long[2..].chars().peekable()));
        assert!(closes_as_arithmetic(" 1 << 2 ))".chars().peekable()));
        // All of it is one word, however it goes on (`x=2claude`, then `-p`).
        assert_eq!(launch("x=$((1+1))claude -p hi"), None);
        // But not what only looks like it: a command substitution, or
        // subshells, of commands.
        assert_eq!(launch("out=$((cd sub && codex exec x) 2>&1)"), fg("codex"));
        assert_eq!(launch("((cd a && claude -p hi) &)"), fg("claude"));
        assert_eq!(launch("n=$(( $(claude -p count) + 1 ))"), fg("claude"));
        assert_eq!(
            launch("echo \"$(( (2+1) << 1 ))\"\ncodex exec x"),
            fg("codex")
        );
    }

    /// R25: a management command doesn't count, even before a real launch
    /// (the first launch in a command is the one recorded).
    #[test]
    fn management_commands_arent_launches() {
        for command in [
            "goose session list",
            "goose session remove -i 3",
            "copilot login",
            "copilot mcp list",
            "copilot run local",
        ] {
            assert_eq!(launch(command), None, "{command}");
        }
        assert_eq!(launch("goose session list; claude -p hi"), fg("claude"));
        // A prompt, recipe or name that happens to be one of those words.
        for command in [
            "goose run -t list",
            "goose run --recipe export",
            "goose session --name list",
            "goose review",
        ] {
            assert_eq!(launch(command), fg("goose"), "{command}");
        }
        assert_eq!(launch("copilot mcp list && claude -p hi"), fg("claude"));
        // An option's value isn't a subcommand: `docs`, `deploy` and `task`
        // here are a folder, a prompt and an agent.
        for command in [
            "copilot --add-dir docs -p 'update the README'",
            "copilot -p deploy",
            "copilot --agent task -p x",
        ] {
            assert_eq!(launch(command), fg("copilot"), "{command}");
        }
    }

    /// R25: a Windows program with its extension matches in any case, and
    /// keeps its own case as `agent-graph run` names it.
    #[test]
    fn windows_programs_keep_their_names() {
        let extra = vec!["MyAgent".to_string()];
        let launch_with = |command: &str| agent_launch(command, &extra).map(|l| l.program);
        assert_eq!(launch_with("MyAgent go").as_deref(), Some("MyAgent"));
        assert_eq!(launch_with("MyAgent.exe go").as_deref(), Some("MyAgent"));
        assert_eq!(launch_with("myagent go"), None);
        assert_eq!(
            launch(r"agent-graph run -- 'C:\bin\Worker.exe'"),
            fg("Worker")
        );
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
