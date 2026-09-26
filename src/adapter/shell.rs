//! Spotting shell commands that start another agent session, like
//! `codex exec "…"` or `claude -p "…"` run from an agent's shell tool.
//!
//! This reads commands the way a shell roughly would: it splits them into
//! simple commands at `;`, `&&`, `||`, `|`, `&` and newlines, outside quotes
//! and comments, reading groups `( … )` and command substitutions as commands
//! of their own (and a `&` as putting the whole list before it in the
//! background), then looks at the program each one runs, past variable
//! assignments and wrappers like `env`, `nohup` or `timeout`.
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

/// Shell keywords that can come before a command (`{` starts a group).
const KEYWORDS: &[&str] = &[
    "do", "then", "else", "if", "elif", "while", "until", "!", "{",
];

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
    "xargs",
    "parallel",
];

/// Wrappers' options that take a value as the next word (`nice -n 10`,
/// `xargs -I {}`), which isn't the program.
const WRAPPER_VALUES: &[(&str, &[&str])] = &[
    (
        "env",
        &[
            "-u",
            "--unset",
            "-C",
            "--chdir",
            "-S",
            "--split-string",
            "-P",
            "-L",
            "-U",
        ],
    ),
    ("nice", &["-n", "--adjustment"]),
    ("exec", &["-a"]),
    (
        "sudo",
        &[
            "-u",
            "--user",
            "-g",
            "--group",
            "-p",
            "--prompt",
            "-C",
            "--close-from",
            "-D",
            "--chdir",
            "-r",
            "--role",
            "-t",
            "--type",
            "-U",
            "--other-user",
            "-T",
            "--command-timeout",
            "-R",
            "--chroot",
            "-h",
            "--host",
        ],
    ),
    ("caffeinate", &["-t", "-w"]),
    ("npx", &["-p", "--package"]),
    ("bunx", &["-p", "--package"]),
    (
        "stdbuf",
        &["-i", "-o", "-e", "--input", "--output", "--error"],
    ),
    ("timeout", &["-s", "--signal", "-k", "--kill-after"]),
    ("time", &["-f", "--format", "-o", "--output"]),
    (
        "xargs",
        &[
            "-I",
            "-J",
            "-L",
            "-P",
            "-R",
            "-S",
            "-E",
            "-a",
            "-d",
            "-n",
            "-s",
            "--arg-file",
            "--delimiter",
            "--max-args",
            "--max-chars",
            "--max-procs",
            "--process-slot-var",
        ],
    ),
    (
        "parallel",
        &[
            "-j",
            "--jobs",
            "-P",
            "-S",
            "--sshlogin",
            "--sshloginfile",
            "-a",
            "--arg-file",
            "-d",
            "--delimiter",
            "-n",
            "-N",
            "--max-args",
            "-L",
            "-E",
            "-I",
            "--colsep",
            "--joblog",
            "--results",
            "--tmpdir",
            "--workdir",
            "--wd",
            "--halt",
            "--retries",
            "--timeout",
            "--delay",
            "--tagstring",
            "--env",
            "--block",
            "--memfree",
            "--load",
        ],
    ),
];

/// A command that starts an agent session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    /// The agent's program, e.g. `codex`. For `agent-graph run -- X`, X's.
    pub program: String,
    /// It's put in the background with `&`, and not waited for later (with
    /// `wait`), so the shell doesn't wait for it.
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

/// Skips `NAME=value` assignments, keywords (and `function NAME`, whose
/// body follows) and wrapper programs (with their options, and `timeout`'s
/// duration) at the start of a simple command.
fn skip_prefixes(words: &[String]) -> &[String] {
    let mut i = 0;
    while let Some(word) = words.get(i) {
        if is_assignment(word) || KEYWORDS.contains(&word.as_str()) {
            i += 1;
            continue;
        }
        if word == "function" {
            i += 2;
            continue;
        }
        let Some(name) = named(word, WRAPPERS) else {
            break;
        };
        let values = WRAPPER_VALUES
            .iter()
            .find(|(w, _)| *w == name)
            .map_or(&[][..], |(_, v)| *v);
        i += 1;
        let options = i;
        while let Some(option) = words.get(i).filter(|w| w.starts_with('-')) {
            i += 1;
            if takes_value(option, values) {
                i += 1;
            }
        }
        // `command -v X` (or `-V`, `-pv`, …) looks X up: it runs nothing.
        let lookup = |o: &String| !o.starts_with("--") && o[1..].contains(['v', 'V']);
        if name == "command" && words[options..i.min(words.len())].iter().any(lookup) {
            return &words[words.len()..];
        }
        if name == "timeout" && words.get(i).is_some() {
            i += 1;
        }
    }
    &words[i.min(words.len())..]
}

/// Whether the word after `option` is its value: it's one of `values`, or
/// a cluster of short options (`-tP`) whose last one takes a value (an
/// earlier one's value is the rest of the cluster: `-oL`).
fn takes_value(option: &str, values: &[&str]) -> bool {
    if values.contains(&option) {
        return true;
    }
    let Some(letters) = option.strip_prefix('-').filter(|l| !l.starts_with('-')) else {
        return false;
    };
    let short = |c: char| {
        values
            .iter()
            .any(|v| v.strip_prefix('-').is_some_and(|l| l.chars().eq([c])))
    };
    letters
        .char_indices()
        .find(|&(_, c)| short(c))
        .is_some_and(|(i, c)| i + c.len_utf8() == letters.len())
}

/// Whether `word` is `NAME=` or `NAME+=`, so a `(` right after it starts
/// an array's values (`arr=(a b)`), not a group.
fn assigns_array(word: &str) -> bool {
    word.strip_suffix('=')
        .map(|name| name.strip_suffix('+').unwrap_or(name))
        .is_some_and(|name| is_assignment(&format!("{name}=")))
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

/// What opened a level of a command: a group of commands, `( … )`, a
/// command substitution, `$( … )` or backticks, or an array's values,
/// `NAME=( … )`, which aren't commands (though substitutions in them are).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Opened {
    #[default]
    Top,
    Group,
    Substitution,
    Backticks,
    Array,
}

/// Compound commands, which a `&` after them puts in the background whole,
/// and the words that end them.
const COMPOUND_OPEN: &[&str] = &["{", "if", "while", "until", "for", "select", "case"];
const COMPOUND_CLOSE: &[&str] = &["}", "fi", "done", "esac"];

/// The simple command being read at one level: the top, or inside a group
/// or substitution, after which the command around it carries on.
#[derive(Default)]
struct Level {
    opened: Opened,
    words: Vec<String>,
    word: String,
    in_word: bool,
    double: bool,
    /// How many of `words` are keywords before the command's first word.
    lead: usize,
    /// `case`s open at this level, whose patterns end with a `)` of their own.
    cases: usize,
    /// `${`s open in `word`.
    braces: usize,
    /// Where this level's commands start in the output.
    start: usize,
    /// Where the list being read (the commands a `&` would put in the
    /// background) starts in the output.
    list: usize,
    /// For each compound command open at this level (`{ … }`, `if … fi`,
    /// …), where the list it's part of starts.
    compounds: Vec<usize>,
    /// The runs of the output this level's shell has put in the background
    /// (as `(start, end)`) and not yet waited for.
    jobs: Vec<(usize, usize)>,
}

impl Level {
    fn end_word(&mut self) {
        if !self.in_word {
            return;
        }
        let word = std::mem::take(&mut self.word);
        if self.lead == self.words.len() && self.opened != Opened::Array {
            // `function NAME` comes before a function's body, as keywords do.
            let name = self.lead > 0 && self.words[self.lead - 1] == "function";
            match word.as_str() {
                "case" => self.cases += 1,
                "esac" => self.cases = self.cases.saturating_sub(1),
                _ => {}
            }
            // A compound command is one command of the list it's in.
            if COMPOUND_OPEN.contains(&word.as_str()) {
                self.compounds.push(self.list);
            } else if COMPOUND_CLOSE.contains(&word.as_str()) {
                if let Some(list) = self.compounds.pop() {
                    self.list = list;
                }
            }
            if name || word == "function" || KEYWORDS.contains(&word.as_str()) {
                self.lead += 1;
            }
        }
        self.words.push(word);
        self.in_word = false;
    }

    fn end_command(&mut self, out: &mut Vec<Vec<String>>) {
        self.end_word();
        if self.opened == Opened::Array {
            self.words.clear();
        } else if !self.words.is_empty() {
            // `wait` waits for the jobs this shell started (`wait $pid`, for
            // one of them), so they're not in the background after all;
            // `wait -n`, for only the first to finish.
            let program = skip_prefixes(&self.words);
            if program.first().is_some_and(|p| p == "wait") && !program.iter().any(|a| a == "-n") {
                self.jobs.clear();
            }
            out.push(std::mem::take(&mut self.words));
        }
        self.lead = 0;
    }

    /// Ends the command, and the list it's in.
    fn end_list(&mut self, out: &mut Vec<Vec<String>>) {
        self.end_command(out);
        self.list = out.len();
    }

    /// Whether what's been read of this command is a function's name, so
    /// that `()` defines it: a name alone (after any keywords, or after
    /// `function`), not a command's argument (`claude -p hi <()`).
    fn names_function(&self) -> bool {
        let rest = self.words.len() - self.lead;
        rest == usize::from(!self.in_word)
    }

    /// Whether a command starts here, after any keywords (or `for`, for
    /// `for ((…))`).
    fn at_command_start(&self) -> bool {
        !self.in_word
            && (self.lead == self.words.len()
                || self.lead + 1 == self.words.len() && self.words[self.lead] == "for")
    }
}

/// Starts reading a group or substitution; the command around it waits.
fn open(level: &mut Level, around: &mut Vec<Level>, opened: Opened, start: usize) {
    let inner = Level {
        opened,
        start,
        list: start,
        ..Level::default()
    };
    around.push(std::mem::replace(level, inner));
}

/// Ends a group or substitution, and carries on with the command around
/// it, where a substitution is (part of) a word. The jobs it didn't wait for
/// go to `kept`: they're its shell's, so the one around it can't wait for
/// them.
fn close(
    level: &mut Level,
    around: &mut Vec<Level>,
    out: &mut Vec<Vec<String>>,
    kept: &mut Vec<(usize, usize)>,
) {
    level.end_command(out);
    kept.append(&mut level.jobs);
    let opened = level.opened;
    let empty = out.len() == level.start;
    let Some(outer) = around.pop() else {
        return;
    };
    *level = outer;
    if opened != Opened::Group {
        level.in_word = true;
    } else if empty && level.names_function() {
        // `NAME()` (or `function NAME()`) defines a function: that runs
        // nothing, and its body is a command of its own.
        level.words.clear();
        level.word.clear();
        level.in_word = false;
        level.lead = 0;
    }
}

/// Splits `command` into simple commands (as words, with quotes removed),
/// each with whether it's put in the background, in the order the shell
/// runs them (a substitution before the command it's in).
fn split(command: &str) -> Vec<(Vec<String>, bool)> {
    let mut out: Vec<Vec<String>> = Vec::new();
    // The runs of `out` put in the background, as (start, end), by shells
    // that have ended without waiting for them.
    let mut jobs: Vec<(usize, usize)> = Vec::new();
    let mut chars = command.chars().peekable();
    let mut single = false;
    let mut cur = Level::default();
    // The levels around this one, innermost last, and how many of them are
    // backticks.
    let mut around: Vec<Level> = Vec::new();
    let mut backticks = 0;
    // Heredocs whose bodies start after this line: (delimiter, `<<-`).
    let mut heredocs: Vec<(String, bool)> = Vec::new();

    while let Some(c) = chars.next() {
        if single {
            if c == '\'' {
                single = false;
            } else {
                cur.word.push(c);
            }
            continue;
        }
        match c {
            '\\' => match chars.next() {
                // A backslash-newline joins the two lines.
                Some('\n') => {}
                Some('\r') if chars.peek() == Some(&'\n') => {
                    chars.next();
                }
                Some(next) => {
                    cur.word.push(next);
                    cur.in_word = true;
                }
                None => {}
            },
            '\'' if !cur.double => {
                single = true;
                cur.in_word = true;
            }
            // `$$` is a parameter (the quote in `$$'…'` is a plain one).
            '$' if chars.peek() == Some(&'$') => {
                chars.next();
                cur.word.push_str("$$");
                cur.in_word = true;
            }
            // `$'…'`: quoted, with escapes (`$'it\'s'`).
            '$' if !cur.double && chars.peek() == Some(&'\'') => {
                chars.next();
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => cur.word.extend(chars.next()),
                        '\'' => break,
                        _ => cur.word.push(c),
                    }
                }
                cur.in_word = true;
            }
            '"' => {
                cur.double = !cur.double;
                cur.in_word = true;
            }
            // `$[…]`: arithmetic, the old way.
            '$' if chars.peek() == Some(&'[') => {
                cur.word.push(c);
                for c in chars.by_ref() {
                    cur.word.push(c);
                    if c == ']' {
                        break;
                    }
                }
                cur.in_word = true;
            }
            // A command substitution runs commands of its own, even inside
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
                    cur.word.push('$');
                    skip_arithmetic(&mut chars, &mut cur.word);
                    cur.in_word = true;
                    continue;
                }
                open(&mut cur, &mut around, Opened::Substitution, out.len());
            }
            '`' if cur.opened == Opened::Backticks => {
                close(&mut cur, &mut around, &mut out, &mut jobs);
                backticks -= 1;
            }
            '`' => {
                open(&mut cur, &mut around, Opened::Backticks, out.len());
                backticks += 1;
            }
            _ if cur.double => {
                cur.word.push(c);
                cur.in_word = true;
            }
            // `${…}` is one word, spaces, `#` and all (`${x:- #}`).
            '$' if chars.peek() == Some(&'{') => {
                chars.next();
                cur.word.push_str("${");
                cur.braces += 1;
                cur.in_word = true;
            }
            '}' if cur.braces > 0 => {
                cur.word.push(c);
                cur.braces -= 1;
            }
            _ if cur.braces > 0 => cur.word.push(c),
            // A comment, to the end of the line (in backticks, or to the one
            // that ends them).
            '#' if !cur.in_word => {
                let ends = |c: &char| *c == '\n' || backticks > 0 && *c == '`';
                while let Some(c) = chars.next_if(|c| !ends(c)) {
                    if c == '\\' && backticks > 0 {
                        chars.next_if(|c| *c != '\n');
                    }
                }
            }
            // `<<-? WORD`: a heredoc, whose body (the lines after this one,
            // up to WORD) is text, not commands. (`<<<` is a here-string:
            // no WORD follows, so no body.)
            '<' if chars.peek() == Some(&'<') => {
                chars.next();
                cur.end_word();
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
            ' ' | '\t' | '\r' => cur.end_word(),
            '\n' => {
                cur.end_list(&mut out);
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
            '(' if chars.peek() == Some(&'(') && cur.at_command_start() && {
                let mut ahead = chars.clone();
                ahead.next();
                closes_as_arithmetic(ahead)
            } =>
            {
                chars.next();
                skip_arithmetic(&mut chars, &mut cur.word);
                cur.in_word = true;
            }
            '(' if cur.in_word && assigns_array(&cur.word) => {
                open(&mut cur, &mut around, Opened::Array, out.len());
            }
            '(' => open(&mut cur, &mut around, Opened::Group, out.len()),
            ')' => {
                cur.end_word();
                // Unless it ends a `case` pattern.
                if cur.cases == 0
                    && matches!(
                        cur.opened,
                        Opened::Group | Opened::Substitution | Opened::Array
                    )
                {
                    close(&mut cur, &mut around, &mut out, &mut jobs);
                } else {
                    cur.end_list(&mut out);
                }
            }
            ';' => cur.end_list(&mut out),
            '|' => {
                chars.next_if_eq(&'|');
                cur.end_command(&mut out);
            }
            // `2>&1`, `&>` and `<&3` redirect: the `&` is part of the word.
            '&' if cur.word.ends_with(['>', '<']) || chars.peek() == Some(&'>') => {
                cur.word.push(c);
                cur.in_word = true;
            }
            // `&&` runs the next command after this one.
            '&' if chars.next_if_eq(&'&').is_some() => cur.end_command(&mut out),
            // A lone `&` puts the list before it in the background, with
            // any groups and substitutions in it.
            '&' => {
                cur.end_command(&mut out);
                cur.jobs.push((cur.list, out.len()));
                cur.list = out.len();
            }
            _ => {
                cur.word.push(c);
                cur.in_word = true;
            }
        }
    }
    // What isn't closed ends here.
    while !around.is_empty() {
        close(&mut cur, &mut around, &mut out, &mut jobs);
    }
    cur.end_command(&mut out);
    jobs.append(&mut cur.jobs);
    // Which commands are in a job: a count of the jobs each starts and ends.
    let mut edges = vec![0i64; out.len() + 1];
    for (start, end) in jobs {
        edges[start] += 1;
        edges[end] -= 1;
    }
    let mut inside = 0;
    out.into_iter()
        .zip(edges)
        .map(|(words, edge)| {
            inside += edge;
            (words, inside > 0)
        })
        .collect()
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
        assert_eq!(launch("! true; (( x <<= 1 ))\nclaude -p hi"), fg("claude"));
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
        assert_eq!(
            launch("((cd a && claude -p hi) &)"),
            Some(("claude".into(), true))
        );
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

    /// R26: a command substitution inside double quotes ends at its own
    /// `)` (or backtick), and the quotes carry on after it.
    #[test]
    fn quotes_carry_on_after_a_substitution() {
        // The commit message Claude writes, then a launch.
        let commit = "git commit -m \"$(cat <<'EOF'\nFix it\nEOF\n)\" && claude -p hi";
        assert_eq!(launch(commit), fg("claude"));
        assert_eq!(launch(r#"echo "$(date)" && codex exec x"#), fg("codex"));
        assert_eq!(launch(r#"echo "`date`"; codex exec x"#), fg("codex"));
        // What comes after it inside the quotes is still quoted.
        assert_eq!(launch(r#"echo "$(date): then claude -p hi""#), None);
        // Its own quotes, parentheses and substitutions.
        assert_eq!(
            launch(r#"x="$(echo "a" ')' "$(date)")"; claude -p hi"#),
            fg("claude")
        );
        assert_eq!(
            launch(r#"x="$( (cd a && pwd) )"; claude -p hi"#),
            fg("claude")
        );
        // Backticks and comments inside one.
        assert_eq!(launch("x=\"$(echo `date`)\"; claude -p hi"), fg("claude"));
        assert_eq!(launch("x=\"$(ls # it's\n)\"; claude -p hi"), fg("claude"));
        // One that never closes ends with the command.
        assert_eq!(launch(r#"claude -p "$(cat x"#), fg("claude"));
        // A `case` pattern's `)` doesn't end it (but only `case` starting a
        // command is one).
        assert_eq!(launch(r#"x="$(echo case)"; claude -p hi"#), fg("claude"));
        assert_eq!(
            launch(r#"x="$(case $1 in a) echo "it's";; esac)"; claude -p hi"#),
            fg("claude")
        );
    }

    /// R26: the command around a substitution carries on after it: it's
    /// one word of that command, not the end of it.
    #[test]
    fn a_substitution_is_a_word() {
        assert_eq!(
            launch(r#"codex exec "$(cat prompt.md)" &"#),
            Some(("codex".into(), true))
        );
        assert_eq!(launch(r#""$(pwd)/bin/claude" -p hi"#), fg("claude"));
        assert_eq!(launch("X=$(date) claude -p hi"), fg("claude"));
        assert_eq!(launch("X=`date` claude -p hi"), fg("claude"));
        // An argument, or the program, but not codex.
        assert_eq!(launch("echo $(date) codex exec x"), None);
        assert_eq!(launch("$(which x) codex exec x"), None);
        assert_eq!(launch("`which x` codex exec x"), None);
        // The shell runs a substitution first, so its launch comes first.
        assert_eq!(launch(r#"claude -p "$(codex exec x)""#), fg("codex"));
        // `( … )` is one too, as a process substitution.
        assert_eq!(launch("diff <(ls a) <(ls b) codex"), None);
    }

    /// R26: a backslash at the end of a line joins it to the next.
    #[test]
    fn line_continuations_join_lines() {
        assert_eq!(launch("cd app && \\\n  claude -p hi"), fg("claude"));
        assert_eq!(launch("FOO=1 \\\n  codex exec x"), fg("codex"));
        assert_eq!(launch("goose \\\n  session"), fg("goose"));
        assert_eq!(launch("goose \\\r\n  session"), fg("goose"));
        assert_eq!(launch("echo \"a \\\nb\" && claude -p hi"), fg("claude"));
    }

    /// R26: a comment is skipped, apostrophes and all.
    #[test]
    fn comments_are_skipped() {
        assert_eq!(
            launch("# don't skip the build\ncargo build && codex exec x"),
            fg("codex")
        );
        assert_eq!(
            launch("cargo build # it's slow\nclaude -p hi"),
            fg("claude")
        );
        assert_eq!(launch("# claude -p hi\nls"), None);
        // `#` inside a word isn't one.
        assert_eq!(launch("echo ${#x} && claude -p hi"), fg("claude"));
        assert_eq!(launch("echo a#b 'c' && claude -p hi"), fg("claude"));
        // In backticks, it ends at the one that ends them.
        assert_eq!(launch("x=`echo 1 #note`; claude -p hi"), fg("claude"));
        assert_eq!(launch("x=`echo 1 #a \\` claude`; echo"), None);
        assert_eq!(launch("x=`echo 1 #a \\\\`; claude -p hi"), fg("claude"));
        assert_eq!(launch("x=`echo 1 #a \\\nclaude -p hi`"), fg("claude"));
        // After a group, it starts one.
        assert_eq!(launch("(cd a)#don't\nclaude -p hi"), fg("claude"));
    }

    /// R26: the other quotes: `$'…'` has escapes, and `${…}` is part of a
    /// word, however it's braced (while `{` on its own starts a group).
    #[test]
    fn other_quotes_are_read_as_the_shell_reads_them() {
        assert_eq!(launch(r"echo $'it\'s' && claude -p hi"), fg("claude"));
        assert_eq!(launch(r#"echo "$'" && claude -p hi"#), fg("claude"));
        assert_eq!(
            launch("claude -p ${PROMPT} &"),
            Some(("claude".into(), true))
        );
        assert_eq!(launch("echo ${x:-a} codex exec x"), None);
        // `${…}` is all one word, spaces, `#` and all.
        assert_eq!(launch("echo ${x:- #}; claude -p hi"), fg("claude"));
        assert_eq!(launch("X=${y:- } claude -p hi"), fg("claude"));
        // `$$` is a parameter: the quote after it is a plain one.
        assert_eq!(launch(r"echo $$'a\'; claude -p hi"), fg("claude"));
        assert_eq!(launch("{ claude -p hi; } > log"), fg("claude"));
    }

    /// R26: a function's body is a command of its own (it runs when the
    /// function is called); defining one runs nothing.
    #[test]
    fn functions_bodies_are_commands() {
        assert_eq!(launch(r#"f() { claude -p "$1"; }; f hi"#), fg("claude"));
        assert_eq!(launch("f(){ claude -p hi; }"), fg("claude"));
        assert_eq!(launch("f ()\n{\n  claude -p hi\n}"), fg("claude"));
        assert_eq!(
            launch(r#"function f { codex exec "$1"; }; f hi"#),
            fg("codex")
        );
        assert_eq!(launch("function f() { codex exec x; }"), fg("codex"));
        assert_eq!(
            launch(r#"run() { codex exec "$1" > "$1.log" 2>&1; }; run a & run b & wait"#),
            fg("codex")
        );
        assert_eq!(launch("codex() { echo hi; }"), None);
        assert_eq!(launch("codex () { echo hi; }"), None);
        assert_eq!(launch("function claude { echo hi; }"), None);
        // Only a name before `()` makes it a definition.
        assert_eq!(launch("claude -p hi <()"), fg("claude"));
        assert_eq!(launch("claude <()"), fg("claude"));
        assert_eq!(
            launch(r#"claude -p "$(cat p.md)" <() &"#),
            Some(("claude".into(), true))
        );
        // A `case` in a function's body is one.
        assert_eq!(
            launch(
                r#"x="$(function f { case $1 in a) echo "it's";; esac; }; f a)"; claude -p hi &"#
            ),
            Some(("claude".into(), true))
        );
    }

    /// R26: a wrapper's option that takes a value (`xargs -I {}`, `nice -n
    /// 10`) isn't the program it runs.
    #[test]
    fn wrappers_option_values_arent_programs() {
        assert_eq!(
            launch(r#"ls *.md | xargs -I{} claude -p "review {}""#),
            fg("claude")
        );
        assert_eq!(
            launch(r#"ls *.md | xargs -P 4 -I {} codex exec "review {}""#),
            fg("codex")
        );
        assert_eq!(launch("nice -n 10 claude -p hi"), fg("claude"));
        assert_eq!(launch("sudo -u bot codex exec x"), fg("codex"));
        assert_eq!(launch("timeout -s KILL 60 claude -p hi"), fg("claude"));
        assert_eq!(launch("/usr/bin/time -o t.log codex exec x"), fg("codex"));
        assert_eq!(launch("npx -p @openai/codex codex exec x"), fg("codex"));
        assert_eq!(launch("parallel -j 4 claude -p {} ::: a b"), fg("claude"));
        assert_eq!(
            launch("parallel --halt now,fail=1 -j4 claude -p {} ::: a b"),
            fg("claude")
        );
        assert_eq!(launch("sudo -R /jail codex exec x"), fg("codex"));
        // Short options together: the last one's value is the next word,
        // an earlier one's is the rest of the word.
        assert_eq!(
            launch(r#"ls | xargs -tP 4 -I{} codex exec "{}""#),
            fg("codex")
        );
        assert_eq!(launch(r#"ls | xargs -rI {} codex exec "{}""#), fg("codex"));
        assert_eq!(
            launch(r#"cat f | xargs -0I {} claude -p "{}""#),
            fg("claude")
        );
        assert_eq!(launch("sudo -Eu bot claude -p hi"), fg("claude"));
        assert_eq!(launch("stdbuf -oL claude -p hi"), fg("claude"));
        assert_eq!(launch("nice -n10 claude -p hi"), fg("claude"));
        // A long option isn't a cluster.
        assert!(takes_value("-xP", &["-P"]));
        assert!(!takes_value("--xP", &["-P"]));
        assert!(!takes_value("-Px", &["-P"]));
    }

    /// R26: splitting takes time in proportion to the command, however many
    /// keywords start it.
    #[test]
    fn splitting_takes_time_in_proportion_to_the_command() {
        let started = std::time::Instant::now();
        for run in ["! ", "{ "] {
            let command = format!("{}claude -p hi", run.repeat(200_000));
            assert_eq!(launch(&command), fg("claude"), "{run:?}");
        }
        for then in ["esac ", "((1)) "] {
            let command = format!("{}{}", "! ".repeat(100_000), then.repeat(100_000));
            assert_eq!(launch(&command), None, "{then:?}");
        }
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
    }

    /// R58: a `&` puts the whole list before it in the background, groups
    /// and all; `<&` is a redirection, and `NAME=(…)` an array's values.
    #[test]
    fn a_background_list_is_all_in_the_background() {
        let bg = |program: &str| Some((program.to_string(), true));
        assert_eq!(launch("(cd x && codex exec y) &"), bg("codex"));
        assert_eq!(launch("{ codex exec y; } &"), bg("codex"));
        assert_eq!(launch("claude -p a && echo done &"), bg("claude"));
        assert_eq!(launch("claude -p a | tee log &"), bg("claude"));
        assert_eq!(launch("echo \"$(claude -p a)\" &"), bg("claude"));
        assert_eq!(
            launch("while read f; do claude -p \"$f\"; done < list &"),
            bg("claude")
        );
        assert_eq!(launch("if true; then codex exec x; fi &"), bg("codex"));
        assert_eq!(launch("cd x && { claude -p a; } > log &"), bg("claude"));
        // Only the list the `&` ends: not one before it, nor one after.
        assert_eq!(launch("claude -p a; echo &"), fg("claude"));
        assert_eq!(launch("claude -p a\necho &"), fg("claude"));
        assert_eq!(launch("echo & claude -p a"), fg("claude"));
        assert_eq!(launch("{ claude -p a; echo & }"), fg("claude"));
        assert_eq!(launch("(claude -p a; echo &)"), fg("claude"));
        // `<&` duplicates a file descriptor: nothing goes in the background.
        assert_eq!(launch("claude -p a 0<&3"), fg("claude"));
        assert_eq!(launch("claude -p a <&- ; echo"), fg("claude"));
        // An array's values aren't commands, but a substitution in them is.
        assert_eq!(launch("arr=(claude codex)"), None);
        assert_eq!(launch("arr+=(claude)\ncodex exec x"), fg("codex"));
        assert_eq!(launch("local arr=(\n  claude\n)"), None);
        assert_eq!(launch("arr=(\"$(claude -p a)\" b)"), fg("claude"));
    }

    /// R42: jobs the command then waits for (`wait`) aren't in the
    /// background: the shell, and the call that ran it, waits for them.
    #[test]
    fn jobs_waited_for_arent_in_the_background() {
        let bg = |program: &str| Some((program.to_string(), true));
        assert_eq!(launch("claude -p a & claude -p b & wait"), fg("claude"));
        assert_eq!(
            launch("for f in *.md; do claude -p \"$f\" & done; wait"),
            fg("claude")
        );
        assert_eq!(
            launch("codex exec a > a.log 2>&1 &\npid=$!\necho started\nwait $pid"),
            fg("codex")
        );
        assert_eq!(
            launch("{ codex exec a & }; if true; then wait; fi"),
            fg("codex")
        );
        // Only jobs started before it, by the same shell (a subshell's jobs
        // aren't its parent's, nor the other way round)...
        assert_eq!(launch("wait; claude -p a &"), bg("claude"));
        assert_eq!(launch("(claude -p a &); wait"), bg("claude"));
        assert_eq!(launch("claude -p a & (wait)"), bg("claude"));
        assert_eq!(launch("claude -p a & x=$(wait)"), bg("claude"));
        // ...and not `wait -n`, which waits for only one of them.
        assert_eq!(launch("claude -p a & claude -p b & wait -n"), bg("claude"));
        assert_eq!(launch("echo wait & claude -p a &"), bg("claude"));
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
