//! Runs the real binary, the way provider hooks and users do.

mod common;

use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use common::fixture;

/// The binary, without any link to a session this test run is inside.
fn bin() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_agent-graph"));
    for var in [
        "AGENT_GRAPH_PARENT",
        "AGENT_GRAPH_PARENT_CODEX",
        "TRACEPARENT",
        "CLAUDE_ENV_FILE",
        "CODEX_THREAD_ID",
        "CODEX_SESSION_ID",
        "AGENT_GRAPH_PARENT_CURSOR",
        "CURSOR_CONVERSATION_ID",
    ] {
        command.env_remove(var);
    }
    // Installing for Codex trusts its hooks in Codex's own config.toml, even
    // for a project's: never the real one.
    command.env("CODEX_HOME", codex_home());
    command
}

/// A Codex home for the tests, shared, so `bin()` never touches the real one.
fn codex_home() -> &'static Path {
    static HOME: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    HOME.get_or_init(|| tempfile::tempdir().unwrap()).path()
}

fn emit(home: &Path, args: &[&str], stdin: &str, extra_env: &[(&str, &str)]) -> Output {
    let mut child = bin()
        .arg("emit")
        .args(args)
        .env("AGENT_GRAPH_HOME", home)
        .envs(extra_env.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn read(path: impl AsRef<Path>) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

#[test]
fn emit_writes_one_line_and_prints_nothing() {
    let home = tempfile::tempdir().unwrap();
    let payload = fixture("claude-code/session.jsonl")[0].to_string();
    let out = emit(home.path(), &["--provider", "claude-code"], &payload, &[]);

    assert!(out.status.success());
    assert!(out.stdout.is_empty(), "stdout must stay empty");
    assert!(out.stderr.is_empty());
    let file = home
        .path()
        .join("events/claude-code-5f2c1e8a-3b4d-4e5f-8a9b-0c1d2e3f4a5b.jsonl");
    let text = read(&file);
    assert_eq!(text.lines().count(), 1);
    assert!(text.contains("\"type\":\"session.started\""));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "events are private to the user");
    }
}

#[test]
fn emit_never_fails_the_hook() {
    let home = tempfile::tempdir().unwrap();
    for (args, stdin) in [
        (vec!["--provider", "claude-code"], "this is not json"),
        (vec!["--provider", "nope"], "{}"),
        (vec![], "{}"),
        (
            vec!["--provider", "claude-code"],
            "{\"hook_event_name\": \"Stop\"}",
        ),
    ] {
        let out = emit(home.path(), &args, stdin, &[]);
        assert_eq!(out.status.code(), Some(0), "args {args:?}");
        assert!(
            out.stdout.is_empty() && out.stderr.is_empty(),
            "args {args:?}"
        );
    }
    let log = read(home.path().join("emit.log"));
    assert_eq!(log.lines().count(), 4, "each failure is logged:\n{log}");
}

#[test]
fn raw_capture_keeps_the_payload() {
    let home = tempfile::tempdir().unwrap();
    let payload = fixture("claude-code/session.jsonl")[1].to_string();
    emit(
        home.path(),
        &["--provider=claude-code"],
        &payload,
        &[("AGENT_GRAPH_RAW", "1")],
    );
    let raw = read(
        home.path()
            .join("raw/claude-code-5f2c1e8a-3b4d-4e5f-8a9b-0c1d2e3f4a5b.jsonl"),
    );
    assert_eq!(raw.trim(), payload);
}

#[test]
fn tree_and_snapshot_show_emitted_sessions() {
    let home = tempfile::tempdir().unwrap();
    for payload in fixture("claude-code/session.jsonl").iter().take(12) {
        emit(
            home.path(),
            &["--provider", "claude-code"],
            &payload.to_string(),
            &[],
        );
    }

    let out = bin()
        .arg("tree")
        .env("AGENT_GRAPH_HOME", home.path())
        .output()
        .unwrap();
    let tree = String::from_utf8(out.stdout).unwrap();
    assert!(out.status.success());
    assert!(
        tree.starts_with("claude-code:5f2c1e8a  app  [working]"),
        "{tree}"
    );
    assert!(
        tree.contains("Explore a1f00d  [completed]  Find the auth middleware"),
        "{tree}"
    );
    assert!(
        tree.contains("general-purpose b2c0de  [working]  Run the test suite"),
        "{tree}"
    );

    let out = bin()
        .args(["snapshot", "--json"])
        .env("AGENT_GRAPH_HOME", home.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    let snapshot: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(snapshot["roots"].as_array().unwrap().len(), 1);
}

#[test]
fn snapshot_draws_the_session_as_an_image() {
    let home = tempfile::tempdir().unwrap();
    for payload in fixture("claude-code/session.jsonl").iter().take(14) {
        emit(
            home.path(),
            &["--provider", "claude-code"],
            &payload.to_string(),
            &[],
        );
    }
    let snapshot = |args: &[&str]| {
        bin()
            .arg("snapshot")
            .args(args)
            .env("AGENT_GRAPH_HOME", home.path())
            .env_remove("CLAUDE_CODE_SESSION_ID")
            .output()
            .unwrap()
    };

    // Default: a new PNG in the data directory; only its path is printed.
    let out = snapshot(&["--session", "5f2c"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let path = String::from_utf8(out.stdout).unwrap();
    let png = std::fs::read(path.trim()).unwrap();
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert!(
        path.trim()
            .starts_with(&home.path().join("images").display().to_string())
    );

    // SVG, with the model's text escaped and the "needs you" callout drawn.
    let svg_path = home.path().join("graph.svg");
    let out = snapshot(&["--out", svg_path.to_str().unwrap(), "--theme", "dark"]);
    assert!(out.status.success());
    let svg = std::fs::read_to_string(&svg_path).unwrap();
    assert!(svg.starts_with("<svg"));
    assert!(svg.contains("Session needs you: Claude needs your permission to use Bash"));
    assert!(svg.contains("Explore a1f00d"));

    // JSON by extension.
    let json_path = home.path().join("state.json");
    assert!(
        snapshot(&["--out", json_path.to_str().unwrap()])
            .status
            .success()
    );
    let graph: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&json_path).unwrap()).unwrap();
    assert_eq!(graph["roots"].as_array().unwrap().len(), 1);

    let out = snapshot(&["--session", "nope"]);
    assert!(!out.status.success());
}

#[test]
fn a_snapshot_never_replaces_one_from_the_same_second() {
    let home = tempfile::tempdir().unwrap();
    for payload in fixture("claude-code/session.jsonl").iter().take(14) {
        emit(
            home.path(),
            &["--provider", "claude-code"],
            &payload.to_string(),
            &[],
        );
    }
    // Snapshots already taken this second and the next few (the name only
    // has seconds, so a second snapshot in time would have the same one).
    let images = home.path().join("images");
    std::fs::create_dir_all(&images).unwrap();
    let now = std::time::SystemTime::now();
    let earlier: Vec<_> = (0..5)
        .map(|s| {
            let stamp = humantime::format_rfc3339_seconds(now + std::time::Duration::from_secs(s))
                .to_string()
                .replace(['-', ':'], "")
                .replace('T', "-")
                .trim_end_matches('Z')
                .to_string();
            let path = images.join(format!("agent-graph-{stamp}.png"));
            std::fs::write(&path, "an earlier snapshot").unwrap();
            path
        })
        .collect();

    let out = bin()
        .args(["snapshot", "--session", "5f2c"])
        .env("AGENT_GRAPH_HOME", home.path())
        .env_remove("CLAUDE_CODE_SESSION_ID")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    for path in &earlier {
        assert_eq!(read(path), "an earlier snapshot");
    }
    let path = String::from_utf8(out.stdout).unwrap();
    let png = std::fs::read(path.trim()).unwrap();
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
}

#[test]
fn install_and_uninstall_project_settings() {
    let project = tempfile::tempdir().unwrap();
    let settings = project.path().join(".claude/settings.json");
    std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
    std::fs::write(&settings, r#"{"model": "opus"}"#).unwrap();

    let run = |args: &[&str]| {
        bin()
            .args(args)
            .current_dir(project.path())
            .output()
            .unwrap()
    };

    let out = run(&["install", "claude-code", "--scope", "project", "--dry-run"]);
    assert!(out.status.success());
    assert_eq!(
        read(&settings),
        r#"{"model": "opus"}"#,
        "dry run writes nothing"
    );

    let out = run(&["install", "claude-code", "--scope", "project"]);
    assert!(
        !out.status.success(),
        "no terminal and no --yes, so it refuses"
    );

    let out = run(&["install", "claude-code", "--scope", "project", "--yes"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let installed: serde_json::Value = serde_json::from_str(&read(&settings)).unwrap();
    assert_eq!(installed["model"], "opus");
    assert!(
        installed["hooks"]["SessionStart"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .ends_with("emit --provider claude-code")
    );
    assert!(
        project
            .path()
            .join(".claude/settings.json.agent-graph.bak")
            .exists()
    );

    let out = run(&["uninstall", "claude-code", "--scope", "project", "--yes"]);
    assert!(out.status.success());
    let restored: serde_json::Value = serde_json::from_str(&read(&settings)).unwrap();
    assert_eq!(restored, serde_json::json!({"model": "opus"}));
    assert!(
        !project.path().join(".claude/skills").exists(),
        "the /agent-graph skill and its folders are gone"
    );
}

#[test]
fn install_adds_the_agent_graph_command() {
    let project = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        bin()
            .args(args)
            .current_dir(project.path())
            .output()
            .unwrap()
    };
    let skill = project.path().join(".claude/skills/agent-graph/SKILL.md");

    assert!(
        run(&["install", "claude-code", "--scope", "project", "--yes"])
            .status
            .success()
    );
    let text = read(&skill);
    assert!(text.starts_with("---\nname: agent-graph\n"));
    assert!(text.contains("agent-graph snapshot --session current"));

    // Without it, only the hooks.
    let other = tempfile::tempdir().unwrap();
    let out = bin()
        .args([
            "install",
            "claude-code",
            "--scope",
            "project",
            "--yes",
            "--no-slash-command",
        ])
        .current_dir(other.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(!other.path().join(".claude/skills").exists());

    // Other agents get just the command, in their own format.
    assert!(
        run(&["install", "gemini", "--scope", "project", "--yes"])
            .status
            .success()
    );
    assert!(read(project.path().join(".gemini/commands/agent-graph.toml")).contains("{{args}}"));
    assert!(
        run(&["install", "codex", "--scope", "project", "--yes"])
            .status
            .success()
    );
    assert!(
        project
            .path()
            .join(".agents/skills/agent-graph/SKILL.md")
            .exists()
    );

    // A command the user wrote themselves is never touched.
    std::fs::write(&skill, "my own skill").unwrap();
    let out = run(&["install", "claude-code", "--scope", "project", "--yes"]);
    assert!(String::from_utf8_lossy(&out.stdout).contains("alone"));
    assert!(
        run(&["uninstall", "claude-code", "--scope", "project", "--yes"])
            .status
            .success()
    );
    assert_eq!(read(&skill), "my own skill");
}

#[test]
fn help_explains_the_program() {
    let long = bin().arg("--help").output().unwrap();
    assert!(long.status.success());
    let text = String::from_utf8(long.stdout).unwrap();
    for part in [
        "Getting started:",
        "Examples:",
        "Where things live:",
        "AGENT_GRAPH_HOME",
        "watch-remote",
    ] {
        assert!(text.contains(part), "--help is missing {part:?}");
    }
    assert!(
        !text.contains("emit"),
        "the hooks' own command stays hidden"
    );

    let short = String::from_utf8(bin().arg("-h").output().unwrap().stdout).unwrap();
    assert!(short.contains("Get started") && !short.contains("Where things live:"));

    // No arguments: the short help, and success.
    let bare = bin().output().unwrap();
    assert!(bare.status.success());
    assert!(
        String::from_utf8(bare.stdout)
            .unwrap()
            .contains("Commands:")
    );

    let install =
        String::from_utf8(bin().args(["install", "--help"]).output().unwrap().stdout).unwrap();
    assert!(install.contains("Examples:") && install.contains("install codex"));
}

// ---------- linking sessions (design §6) ----------

fn events_in(home: &Path) -> Vec<serde_json::Value> {
    let mut files: Vec<_> = std::fs::read_dir(home.join("events"))
        .map(|d| d.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    files.sort();
    files
        .iter()
        .flat_map(|f| {
            read(f)
                .lines()
                .map(|l| serde_json::from_str(l).unwrap())
                .collect::<Vec<_>>()
        })
        .collect()
}

#[test]
fn a_starting_session_links_to_its_parent_and_passes_itself_on() {
    let home = tempfile::tempdir().unwrap();
    let env_file = home.path().join("claude-env.sh");
    let inherited = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";
    let payload = fixture("claude-code/session.jsonl")[0].to_string();
    let out = emit(
        home.path(),
        &["--provider", "claude-code"],
        &payload,
        &[
            ("AGENT_GRAPH_PARENT", "claude-code:the-parent"),
            ("TRACEPARENT", inherited),
            ("CLAUDE_ENV_FILE", env_file.to_str().unwrap()),
        ],
    );
    assert!(out.status.success() && out.stdout.is_empty());

    let events = events_in(home.path());
    let started = &events[0];
    assert_eq!(started["parent"], "claude-code:the-parent");
    assert_eq!(started["data"]["link_method"], "env");
    let traceparent = started["trace"]["traceparent"].as_str().unwrap();
    assert!(
        traceparent.starts_with("00-0af7651916cd43dd8448eb211c80319c-"),
        "continues the trace: {traceparent}"
    );
    if cfg!(any(target_os = "linux", target_os = "macos", windows)) {
        let process = started["data"]["process"]
            .as_str()
            .expect("its agent's process");
        assert!(process.contains('@'));
        // The "agent" here is this test, which ran the hook.
        assert!(process.starts_with(&format!("{}@", std::process::id())));
    }

    let node = "claude-code:5f2c1e8a-3b4d-4e5f-8a9b-0c1d2e3f4a5b";
    assert_eq!(
        read(&env_file),
        format!(
            "export AGENT_GRAPH_PARENT='{node}'\nexport AGENT_GRAPH_PARENT_CODEX=''\nexport AGENT_GRAPH_PARENT_CURSOR=''\nexport TRACEPARENT='{traceparent}'\n"
        ),
        "sessions it starts will link back to it"
    );
}

#[test]
fn a_session_is_not_its_own_parent() {
    let home = tempfile::tempdir().unwrap();
    let payload = fixture("claude-code/session.jsonl")[0].to_string();
    let own = "claude-code:5f2c1e8a-3b4d-4e5f-8a9b-0c1d2e3f4a5b";
    emit(
        home.path(),
        &["--provider", "claude-code"],
        &payload,
        &[("AGENT_GRAPH_PARENT", own)],
    );
    let events = events_in(home.path());
    assert!(events[0].get("parent").is_none());
    assert!(events[0]["data"].get("link_method").is_none());
}

/// A command that prints `AGENT_GRAPH_PARENT` and exits with `code`.
fn print_parent_and_exit(code: u8) -> Vec<String> {
    if cfg!(windows) {
        vec![
            "cmd".into(),
            "/C".into(),
            format!("echo %AGENT_GRAPH_PARENT%& exit {code}"),
        ]
    } else {
        vec![
            "sh".into(),
            "-c".into(),
            format!("echo \"$AGENT_GRAPH_PARENT\"; exit {code}"),
        ]
    }
}

#[test]
fn run_puts_a_command_in_the_graph_and_passes_on_its_exit_code() {
    let home = tempfile::tempdir().unwrap();
    let out = bin()
        .args(["run", "--name", "worker", "--"])
        .args(print_parent_and_exit(3))
        .env("AGENT_GRAPH_HOME", home.path())
        .env("AGENT_GRAPH_PARENT", "claude-code:outer")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3));

    let events = events_in(home.path());
    let node = events[0]["node"].as_str().unwrap();
    assert!(node.starts_with("run:"));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        node,
        "the command can link what it starts to the run"
    );
    let kinds: Vec<&str> = events.iter().map(|e| e["type"].as_str().unwrap()).collect();
    assert_eq!(
        kinds,
        ["session.started", "status", "status", "session.ended"]
    );
    assert_eq!(events[0]["parent"], "claude-code:outer");
    assert_eq!(events[0]["data"]["title"], "worker");
    assert_eq!(events[2]["data"]["state"], "failed");
    assert_eq!(events[2]["data"]["summary"], "Exited with code 3");

    let tree = bin()
        .args(["tree", "--all"])
        .env("AGENT_GRAPH_HOME", home.path())
        .output()
        .unwrap();
    let tree = String::from_utf8_lossy(&tree.stdout);
    assert!(tree.contains("worker"), "{tree}");
}

#[test]
fn run_succeeds_quietly_and_reports_a_missing_program() {
    let home = tempfile::tempdir().unwrap();
    let ok = bin()
        .args(["run", "--"])
        .args(print_parent_and_exit(0))
        .env("AGENT_GRAPH_HOME", home.path())
        .output()
        .unwrap();
    assert!(ok.status.success());
    assert!(
        ok.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&ok.stderr)
    );
    let events = events_in(home.path());
    assert_eq!(events.len(), 3, "started, working, ended");
    assert!(events[0].get("parent").is_none());

    let home = tempfile::tempdir().unwrap();
    let missing = bin()
        .args(["run", "--", "agent-graph-no-such-program"])
        .env("AGENT_GRAPH_HOME", home.path())
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(127));
    assert!(
        String::from_utf8_lossy(&missing.stderr).contains("can't run agent-graph-no-such-program")
    );
    let events = events_in(home.path());
    assert_eq!(events[2]["data"]["state"], "failed");
}

// ---------- review fixes ----------

/// R1: a repo can ship symlinks where `install --scope project` writes. None
/// of them may lead a write outside the project.
#[cfg(unix)]
#[test]
fn install_never_writes_through_links_a_project_plants() {
    use std::os::unix::fs::symlink;

    let install = |project: &Path| {
        bin()
            .args(["install", "claude-code", "--scope", "project", "--yes"])
            .current_dir(project)
            .output()
            .unwrap()
    };
    let outside = |dir: &Path| {
        let file = dir.join("outside.txt");
        std::fs::write(&file, "keep me").unwrap();
        file
    };

    // A link where the temporary file, or the backup, would go.
    for planted in ["settings.tmp", "settings.json.agent-graph.bak"] {
        let root = tempfile::tempdir().unwrap();
        let victim = outside(root.path());
        let project = root.path().join("repo");
        let claude = project.join(".claude");
        std::fs::create_dir_all(&claude).unwrap();
        std::fs::write(claude.join("settings.json"), r#"{"x": "$(touch pwned)"}"#).unwrap();
        symlink(&victim, claude.join(planted)).unwrap();

        let out = install(&project);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(read(&victim), "keep me", "{planted} led the write outside");
        let installed: serde_json::Value =
            serde_json::from_str(&read(claude.join("settings.json"))).unwrap();
        assert!(installed["hooks"].is_object(), "installed all the same");
    }

    // The settings file itself, or the .claude folder, is a link out. The
    // outside file is JSON, so reading it through the link would work: it
    // mustn't be read, rewritten, or copied into the project as a backup.
    let root = tempfile::tempdir().unwrap();
    let secret = root.path().join("secret.json");
    std::fs::write(&secret, r#"{"token": "s3cret"}"#).unwrap();
    let project = root.path().join("repo");
    std::fs::create_dir_all(project.join(".claude")).unwrap();
    symlink(&secret, project.join(".claude/settings.json")).unwrap();
    let out = install(&project);
    assert!(!out.status.success(), "refuses a linked settings file");
    assert!(String::from_utf8_lossy(&out.stderr).contains("link"));
    assert_eq!(read(&secret), r#"{"token": "s3cret"}"#);
    let copied = std::fs::read_dir(project.join(".claude"))
        .unwrap()
        .flatten()
        .any(|e| read(e.path()).contains("s3cret") && e.file_name() != "settings.json");
    assert!(!copied, "the outside file wasn't copied into the project");

    let root = tempfile::tempdir().unwrap();
    let elsewhere = root.path().join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let project = root.path().join("repo");
    std::fs::create_dir_all(&project).unwrap();
    symlink(&elsewhere, project.join(".claude")).unwrap();
    let out = install(&project);
    assert!(!out.status.success(), "refuses a linked .claude folder");
    assert_eq!(
        std::fs::read_dir(&elsewhere).unwrap().count(),
        0,
        "nothing written outside the project"
    );
}

/// R2: `--out` names a file to create. It never replaces one (unless
/// `--force`), and only writes the formats it knows.
#[test]
fn snapshot_out_never_replaces_a_file_unless_forced() {
    let home = tempfile::tempdir().unwrap();
    for payload in fixture("claude-code/session.jsonl").iter().take(5) {
        emit(
            home.path(),
            &["--provider", "claude-code"],
            &payload.to_string(),
            &[],
        );
    }
    let dir = tempfile::tempdir().unwrap();
    let snapshot = |args: &[&str]| {
        bin()
            .arg("snapshot")
            .args(args)
            .env("AGENT_GRAPH_HOME", home.path())
            .env_remove("CLAUDE_CODE_SESSION_ID")
            .current_dir(dir.path())
            .output()
            .unwrap()
    };
    for name in ["mine.json", "mine.png", "mine.svg"] {
        std::fs::write(dir.path().join(name), "precious").unwrap();
        let out = snapshot(&["--session", "5f2c", "--out", name]);
        assert!(!out.status.success(), "{name} was replaced");
        assert!(String::from_utf8_lossy(&out.stderr).contains("--force"));
        assert_eq!(read(dir.path().join(name)), "precious");
    }
    let out = snapshot(&["--json", "--out", "mine.json"]);
    assert!(!out.status.success());
    assert_eq!(read(dir.path().join("mine.json")), "precious");

    let out = snapshot(&["--json", "--out", "mine.json", "--force"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(read(dir.path().join("mine.json")).contains("\"nodes\""));

    for name in [".bashrc", "notes.txt", "graph"] {
        let out = snapshot(&["--session", "5f2c", "--out", name]);
        assert!(!out.status.success(), "wrote {name}");
        assert!(!dir.path().join(name).exists());
    }

    let out = snapshot(&["--session", "5f2c", "--out", "new.svg"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(read(dir.path().join("new.svg")).starts_with("<svg"));
}

/// R5: a command file that isn't ours stays untouched, even if it isn't
/// UTF-8 (a Windows editor's encoding) or can't be read at all.
#[test]
fn install_leaves_a_command_file_it_cant_read_alone() {
    let project = tempfile::tempdir().unwrap();
    let toml = project.path().join(".gemini/commands/agent-graph.toml");
    std::fs::create_dir_all(toml.parent().unwrap()).unwrap();
    let mine: &[u8] = b"description = \"r\xe9sum\xe9 of my agents\"\nprompt = \"mine\"\n";
    std::fs::write(&toml, mine).unwrap();

    let run = |args: &[&str]| {
        bin()
            .args(args)
            .current_dir(project.path())
            .output()
            .unwrap()
    };
    let out = run(&["install", "gemini", "--scope", "project", "--yes"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        std::fs::read(&toml).unwrap() == mine,
        "replaced the user's file"
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("alone"));

    let out = run(&["uninstall", "gemini", "--scope", "project", "--yes"]);
    assert!(out.status.success());
    assert!(
        std::fs::read(&toml).unwrap() == mine,
        "removed the user's file"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let skill = project.path().join(".claude/skills/agent-graph/SKILL.md");
        std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
        std::fs::write(&skill, "my own skill").unwrap();
        std::fs::set_permissions(&skill, std::fs::Permissions::from_mode(0o000)).unwrap();
        let unreadable = std::fs::read(&skill).is_err(); // not when run as root
        let out = run(&["install", "claude-code", "--scope", "project", "--yes"]);
        std::fs::set_permissions(&skill, std::fs::Permissions::from_mode(0o600)).unwrap();
        if unreadable {
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            assert_eq!(
                read(&skill),
                "my own skill",
                "replaced a skill it couldn't read"
            );
            assert!(String::from_utf8_lossy(&out.stdout).contains("alone"));
        }
    }
}

/// R6: settings often hold API keys. Rewriting them keeps them private, and a
/// settings file that's a link (dotfiles) is updated where it really lives.
#[cfg(unix)]
#[test]
fn install_keeps_settings_private_and_follows_the_users_own_link() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    let install = |config: &Path, home: &Path| {
        bin()
            .args(["install", "claude-code", "--yes", "--no-slash-command"])
            .env("CLAUDE_CONFIG_DIR", config)
            .env("HOME", home)
            .output()
            .unwrap()
    };

    // A private settings file stays private, and so does its backup.
    let home = tempfile::tempdir().unwrap();
    let config = home.path().join(".claude");
    std::fs::create_dir_all(&config).unwrap();
    let settings = config.join("settings.json");
    std::fs::write(&settings, r#"{"env": {"API_KEY": "secret"}}"#).unwrap();
    std::fs::set_permissions(&settings, std::fs::Permissions::from_mode(0o600)).unwrap();
    let out = install(&config, home.path());
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(mode(&settings), 0o600, "settings became readable by others");
    assert_eq!(mode(&config.join("settings.json.agent-graph.bak")), 0o600);

    // Group-readable stays group-readable, and so does the backup (of a
    // file that doesn't have the hooks yet, so it's rewritten).
    std::fs::write(&settings, r#"{"env": {"API_KEY": "other"}}"#).unwrap();
    std::fs::set_permissions(&settings, std::fs::Permissions::from_mode(0o640)).unwrap();
    let out = install(&config, home.path());
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(mode(&settings), 0o640);
    assert_eq!(mode(&config.join("settings.json.agent-graph.bak")), 0o640);

    // A new settings file is private too.
    let home = tempfile::tempdir().unwrap();
    let config = home.path().join(".claude");
    let out = install(&config, home.path());
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(mode(&config.join("settings.json")), 0o600);

    // A link into the user's dotfiles stays a link, and its target gets the
    // hooks, keeping its mode.
    let home = tempfile::tempdir().unwrap();
    let config = home.path().join(".claude");
    let dotfiles = home.path().join("dotfiles");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::create_dir_all(&dotfiles).unwrap();
    let real = dotfiles.join("claude-settings.json");
    std::fs::write(&real, r#"{"model": "opus"}"#).unwrap();
    std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o600)).unwrap();
    let link = config.join("settings.json");
    symlink(&real, &link).unwrap();
    let out = install(&config, home.path());
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    let updated: serde_json::Value = serde_json::from_str(&read(&real)).unwrap();
    assert_eq!(updated["model"], "opus");
    assert!(updated["hooks"].is_object(), "the real file has the hooks");
    assert_eq!(mode(&real), 0o600);
    assert_eq!(
        read(config.join("settings.json.agent-graph.bak")),
        r#"{"model": "opus"}"#,
        "backed up beside the link, not into the dotfiles"
    );
    assert_eq!(std::fs::read_dir(&dotfiles).unwrap().count(), 1);
}

/// R48: `tail` puts the terminal back (out of raw mode and the alternate
/// screen) however it's stopped, then dies of the signal as it would have.
#[cfg(unix)]
#[test]
fn tail_puts_the_terminal_back_when_killed() {
    use std::io::Read;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::process::{CommandExt, ExitStatusExt};

    let home = tempfile::tempdir().unwrap();
    for payload in fixture("claude-code/session.jsonl").iter().take(14) {
        emit(
            home.path(),
            &["--provider", "claude-code"],
            &payload.to_string(),
            &[],
        );
    }
    for sig in [libc::SIGTERM, libc::SIGHUP, libc::SIGINT] {
        // A terminal of its own: `tail` is in the session it controls.
        let (mut master, mut slave) = (0, 0);
        let mut size = libc::winsize {
            ws_row: 24,
            ws_col: 80,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // SAFETY: openpty fills in two new descriptors, which we then own.
        let (master, slave) = unsafe {
            assert_eq!(
                libc::openpty(
                    &mut master,
                    &mut slave,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    // A raw pointer: it's `*const` on Linux, `*mut` on macOS.
                    std::ptr::addr_of_mut!(size),
                ),
                0
            );
            (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave))
        };
        let cooked = |fd: &OwnedFd| {
            // SAFETY: reads the terminal's settings into a zeroed struct.
            unsafe {
                let mut t: libc::termios = std::mem::zeroed();
                assert_eq!(libc::tcgetattr(fd.as_raw_fd(), &mut t), 0);
                t.c_lflag & (libc::ICANON | libc::ECHO) == (libc::ICANON | libc::ECHO)
            }
        };
        assert!(cooked(&master));

        let mut command = bin();
        command
            .arg("tail")
            .env("AGENT_GRAPH_HOME", home.path())
            .stdin(slave.try_clone().unwrap())
            .stdout(slave.try_clone().unwrap())
            .stderr(slave.try_clone().unwrap());
        // SAFETY: setsid and ioctl are async-signal-safe.
        unsafe {
            command.pre_exec(|| {
                libc::setsid();
                libc::ioctl(0, libc::TIOCSCTTY as _, 0);
                Ok(())
            });
        }
        let mut child = command.spawn().unwrap();

        // Everything it draws is read, so it never waits on a full terminal.
        let mut reader = std::fs::File::from(master.try_clone().unwrap());
        let (drawn, output) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = [0; 4096];
            while let Ok(n @ 1..) = reader.read(&mut buf) {
                if drawn.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
        // Wait for it to take over the screen, then kill it.
        let mut seen = Vec::new();
        while !seen.windows(8).any(|w| w == b"\x1b[?1049h") {
            match output.recv_timeout(std::time::Duration::from_secs(10)) {
                Ok(bytes) => seen.extend(bytes),
                Err(_) => {
                    let _ = child.kill();
                    panic!("tail never started: {:?}", String::from_utf8_lossy(&seen));
                }
            }
        }
        assert!(!cooked(&master), "tail is in raw mode");
        // SAFETY: signals the child we started.
        unsafe { libc::kill(child.id() as i32, sig) };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if std::time::Instant::now() > deadline {
                let _ = child.kill();
                panic!("signal {sig} didn't stop tail");
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        };

        assert!(
            cooked(&master),
            "signal {sig} left the terminal in raw mode"
        );
        assert_eq!(status.signal(), Some(sig), "{status:?}");
    }
}

/// R9: a signal `run` was started with ignored (`nohup`, a background job)
/// stays ignored for the command, as it would without `run`.
#[cfg(unix)]
#[test]
fn run_keeps_signals_that_were_ignored_ignored() {
    let home = tempfile::tempdir().unwrap();
    let bin = env!("CARGO_BIN_EXE_agent-graph");
    for sig in ["HUP", "INT"] {
        // The command signals itself: ignored, it carries on.
        let script = format!(
            r#"trap "" {sig}; exec "{bin}" run -- /bin/sh -c 'kill -{sig} $$; echo alive'"#
        );
        let out = Command::new("/bin/sh")
            .args(["-c", &script])
            .env("AGENT_GRAPH_HOME", home.path())
            .env_remove("AGENT_GRAPH_PARENT")
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).trim(),
            "alive",
            "SIG{sig} wasn't left ignored: {:?}",
            out.status
        );
        assert!(out.status.success(), "{sig}: {:?}", out.status);
    }
}

/// R11: on Windows, a program installed as a `.cmd` file (as npm installs
/// codex, gemini and often claude) is found by its bare name.
#[cfg(windows)]
#[test]
fn run_starts_a_cmd_file_by_its_bare_name() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("agentgraphfake.cmd"), "@exit /b 3\r\n").unwrap();
    let path = std::env::join_paths(std::iter::once(dir.path().to_path_buf()).chain(
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
    ))
    .unwrap();
    let home = tempfile::tempdir().unwrap();
    let out = bin()
        .args(["run", "--", "agentgraphfake"])
        .env("PATH", path)
        .env("AGENT_GRAPH_HOME", home.path())
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// PATH without this build's folder, and with `first` (if any) at the front.
fn path_with(first: Option<&Path>) -> std::ffi::OsString {
    let ours = Path::new(env!("CARGO_BIN_EXE_agent-graph"))
        .parent()
        .unwrap()
        .to_path_buf();
    let rest = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .filter(|p| *p != ours)
        .collect::<Vec<_>>();
    std::env::join_paths(first.map(Path::to_path_buf).into_iter().chain(rest)).unwrap()
}

/// R14: project settings are meant to be committed, so their hooks run
/// `agent-graph` from PATH, not this machine's copy; local settings (never
/// committed) keep this copy's full path, unless PATH's is this one.
#[test]
fn project_hooks_run_agent_graph_from_path() {
    let project = tempfile::tempdir().unwrap();
    let install = |scope: &str, path: std::ffi::OsString| {
        let out = bin()
            .args([
                "install",
                "claude-code",
                "--scope",
                scope,
                "--yes",
                "--no-slash-command",
            ])
            .current_dir(project.path())
            .env("PATH", path)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    let command = |file: &str| -> String {
        let settings: serde_json::Value =
            serde_json::from_str(&read(project.path().join(".claude").join(file))).unwrap();
        settings["hooks"]["SessionStart"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let plain = "agent-graph emit --provider claude-code";

    let said = install("project", path_with(None));
    assert_eq!(command("settings.json"), plain);
    assert!(said.contains("PATH"), "says who needs it: {said}");

    install("local", path_with(None));
    let local = command("settings.local.json");
    assert!(
        local.starts_with('"') && local.ends_with("\" emit --provider claude-code"),
        "{local}"
    );
}

/// R14: your own settings keep this copy's full path even when PATH has
/// it: hooks run with Claude Code's own PATH (from a launcher or a
/// scheduler it may be just /usr/bin:/bin), which a plain name would miss.
#[test]
fn user_hooks_keep_the_full_path_even_when_path_has_this_copy() {
    let home = tempfile::tempdir().unwrap();
    let ours = Path::new(env!("CARGO_BIN_EXE_agent-graph"))
        .parent()
        .unwrap();
    let out = bin()
        .args(["install", "claude-code", "--yes", "--no-slash-command"])
        .env("CLAUDE_CONFIG_DIR", home.path().join(".claude"))
        .env("HOME", home.path())
        .env("PATH", path_with(Some(ours)))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let settings: serde_json::Value =
        serde_json::from_str(&read(home.path().join(".claude/settings.json"))).unwrap();
    let command = settings["hooks"]["SessionStart"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap();
    assert!(command.starts_with('"'), "{command}");
}

/// R61: a package manager keeps the program in a folder named for its
/// version and links to it from PATH; the hooks name the link, which an
/// upgrade keeps, not the versioned file, which it removes.
#[cfg(unix)]
#[test]
fn user_hooks_name_the_link_on_path_not_the_versioned_file() {
    let home = tempfile::tempdir().unwrap();
    let cellar = home.path().join("Cellar/agent-graph/0.1.0/bin");
    let bin_dir = home.path().join("bin");
    std::fs::create_dir_all(&cellar).unwrap();
    std::fs::create_dir_all(&bin_dir).unwrap();
    let versioned = cellar.join("agent-graph");
    std::fs::copy(env!("CARGO_BIN_EXE_agent-graph"), &versioned).unwrap();
    let link = bin_dir.join("agent-graph");
    std::os::unix::fs::symlink(&versioned, &link).unwrap();

    // Run by its versioned path, as a hook recorded before this fix, or a
    // wrapper that resolves links, would.
    let mut command = Command::new(&versioned);
    for var in ["AGENT_GRAPH_PARENT", "TRACEPARENT", "CLAUDE_ENV_FILE"] {
        command.env_remove(var);
    }
    let out = command
        .args(["install", "claude-code", "--yes", "--no-slash-command"])
        .env("CLAUDE_CONFIG_DIR", home.path().join(".claude"))
        .env("HOME", home.path())
        .env("AGENT_GRAPH_HOME", home.path().join(".agent-graph"))
        .env("PATH", path_with(Some(&bin_dir)))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let settings: serde_json::Value =
        serde_json::from_str(&read(home.path().join(".claude/settings.json"))).unwrap();
    let command = settings["hooks"]["SessionStart"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap();
    assert_eq!(
        command,
        format!("\"{}\" emit --provider claude-code", link.display())
    );
}

/// R14: installing hooks that run `agent-graph` from PATH says so when
/// PATH has no `agent-graph` (they'd never run for this user).
#[test]
fn project_install_warns_when_agent_graph_isnt_on_path() {
    let project = tempfile::tempdir().unwrap();
    let out = bin()
        .args([
            "install",
            "claude-code",
            "--scope",
            "project",
            "--yes",
            "--no-slash-command",
        ])
        .current_dir(project.path())
        .env("PATH", path_with(None))
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&out.stdout);
    let has_other = std::env::split_paths(&path_with(None)).any(|d| {
        d.join(if cfg!(windows) {
            "agent-graph.exe"
        } else {
            "agent-graph"
        })
        .is_file()
    });
    if has_other {
        assert!(said.contains("different copy"), "{said}");
    } else {
        assert!(said.contains("isn't on your PATH"), "{said}");
    }
}

/// R30: with --scope local, the command still goes in the project's folder
/// (there's nowhere else for it), which may be the project's own, committed
/// one (--scope project): uninstalling locally leaves that one, and says so.
#[test]
fn local_uninstall_leaves_the_projects_committed_command() {
    let project = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let run = |program: &str, args: &[&str], env: &[(&str, &Path)]| {
        let mut command = if program == "agent-graph" {
            bin()
        } else {
            Command::new(program)
        };
        for var in GIT_REPO_VARS {
            command.env_remove(var);
        }
        let out = command
            .args(args)
            .current_dir(project.path())
            .env("HOME", home.path())
            .env("CLAUDE_CONFIG_DIR", home.path().join(".claude"))
            .env("GIT_CONFIG_GLOBAL", home.path().join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .envs(env.iter().copied())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{program} {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    let git = |args: &[&str]| run("git", args, &[]);
    let agent_graph = |args: &[&str], env: &[(&str, &Path)]| run("agent-graph", args, env);
    let install = ["install", "claude-code", "--yes"];
    let uninstall = ["uninstall", "claude-code", "--scope", "local", "--yes"];
    let skill = project.path().join(".claude/skills/agent-graph/SKILL.md");

    // Installed locally, and not committed (staged, say): uninstalling
    // locally removes it.
    git(&["init", "-q"]);
    agent_graph(&[&install[..], &["--scope", "local"]].concat(), &[]);
    assert!(skill.exists());
    git(&["add", "."]);
    agent_graph(&uninstall, &[]);
    assert!(!skill.exists());

    // The project's, committed: kept, whatever repository the environment
    // names (a git hook's, say).
    agent_graph(&[&install[..], &["--scope", "project"]].concat(), &[]);
    git(&["add", "."]);
    git(&[
        "-c",
        "user.name=t",
        "-c",
        "user.email=t@example.com",
        "commit",
        "-qm",
        "x",
    ]);
    git(&["init", "-q", other.path().to_str().unwrap()]);
    let said = agent_graph(&uninstall, &[("GIT_DIR", &other.path().join(".git"))]);
    assert!(skill.exists(), "the project's command is kept");
    assert!(
        said.contains("--scope project"),
        "and says how to remove it: {said}"
    );
    assert!(
        !said.contains("nothing of Agent Graph's is installed"),
        "{said}"
    );

    // When git can't say (it refuses a repository someone else owns, say),
    // it's kept, and says why.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let bin_dir = home.path().join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let fake = bin_dir.join("git");
        std::fs::write(
            &fake,
            "#!/bin/sh\necho 'fatal: detected dubious ownership in repository at /x' >&2\necho 'To add an exception for this directory, call:' >&2\necho '' >&2\necho '\tgit config --global --add safe.directory /x' >&2\nexit 128\n",
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let said = agent_graph(&uninstall, &[("PATH", &bin_dir)]);
        assert!(skill.exists(), "kept");
        assert!(said.contains("dubious ownership"), "{said}");
        assert!(
            said.contains("--scope project"),
            "says how to remove it: {said}"
        );
        assert!(
            said.contains("safe.directory /x\n"),
            "git's command, as it is: {said}"
        );
    }
}

/// Variables that point git at another repository.
const GIT_REPO_VARS: [&str; 5] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_COMMON_DIR",
];

/// R31: `snapshot --session X` (or --all) as JSON is that session (or
/// those), as a picture would be; with neither, it's the whole graph.
#[test]
fn snapshot_json_is_the_sessions_asked_for() {
    let home = tempfile::tempdir().unwrap();
    let payloads = fixture("claude-code/session.jsonl");
    for payload in payloads.iter().take(14) {
        emit(
            home.path(),
            &["--provider", "claude-code"],
            &payload.to_string(),
            &[],
        );
    }
    // Another session, from long ago.
    let mut other = payloads[0].clone();
    other["session_id"] = "0ther-session".into();
    emit(
        home.path(),
        &["--provider", "claude-code"],
        &other.to_string(),
        &[],
    );
    let json = |args: &[&str]| -> serde_json::Value {
        let path = home.path().join(format!("out{}.json", args.join("_")));
        let out = bin()
            .arg("snapshot")
            .args(args)
            .arg("--out")
            .arg(&path)
            .env("AGENT_GRAPH_HOME", home.path())
            .env_remove("CLAUDE_CODE_SESSION_ID")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap()
    };
    // Ids are the provider's, prefixed.
    let own = |id: &str| id.rsplit(['/', ':']).next().unwrap().to_string();
    let has = |graph: &serde_json::Value, id: &str| {
        graph["nodes"]
            .as_object()
            .unwrap()
            .keys()
            .any(|k| own(k).starts_with(id))
    };

    let whole = json(&[]);
    assert_eq!(whole["roots"].as_array().unwrap().len(), 2);

    let one = json(&["--session", "5f2c"]);
    let roots = one["roots"].as_array().unwrap();
    assert_eq!(roots.len(), 1);
    assert!(own(roots[0].as_str().unwrap()).starts_with("5f2c"));
    assert!(has(&one, "5f2c"), "the session");
    assert!(
        one["nodes"].as_object().unwrap().len() > 1,
        "and its agents"
    );
    assert!(!has(&one, "0ther"), "not the other: {one}");

    let other = json(&["--session", "0ther"]);
    assert_eq!(other["nodes"].as_object().unwrap().len(), 1);
}

/// R31: asked for some sessions as data with none recorded, it says so, as
/// a picture does; the whole graph, empty, is fine.
#[test]
fn snapshot_json_of_sessions_needs_some() {
    let home = tempfile::tempdir().unwrap();
    let snapshot = |args: &[&str]| {
        bin()
            .arg("snapshot")
            .args(args)
            .current_dir(home.path())
            .env("AGENT_GRAPH_HOME", home.path())
            .env_remove("CLAUDE_CODE_SESSION_ID")
            .output()
            .unwrap()
    };
    for args in [["--session", "current"].as_slice(), &["--all"]] {
        let out = snapshot(&[args, &["--out", "some.json", "--force"]].concat());
        assert!(!out.status.success(), "{args:?}");
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("no sessions recorded yet"),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    assert!(snapshot(&["--out", "all.json"]).status.success());
}

/// R49: the backup is of the settings as they were without Agent Graph.
/// Installing again, or uninstalling, doesn't replace it with a copy that
/// has the hooks.
#[test]
fn the_settings_backup_keeps_the_original() {
    let project = tempfile::tempdir().unwrap();
    let settings = project.path().join(".claude/settings.json");
    let backup = project.path().join(".claude/settings.json.agent-graph.bak");
    std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
    std::fs::write(&settings, r#"{"model": "opus"}"#).unwrap();
    let run = |args: &[&str]| {
        let out = bin()
            .args(args)
            .args(["--scope", "project", "--yes"])
            .current_dir(project.path())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };

    run(&["install", "claude-code"]);
    assert_eq!(read(&backup), r#"{"model": "opus"}"#);
    // A different hook command, so the settings change again.
    run(&[
        "install",
        "claude-code",
        "--command",
        "ag emit --provider claude-code",
    ]);
    assert_eq!(read(&backup), r#"{"model": "opus"}"#, "installing again");
    run(&["uninstall", "claude-code"]);
    assert_eq!(read(&backup), r#"{"model": "opus"}"#, "uninstalling");

    // Without the hooks, the settings are the user's own again, and those
    // are what the next install backs up.
    std::fs::write(&settings, r#"{"model": "sonnet"}"#).unwrap();
    run(&["install", "claude-code"]);
    assert_eq!(read(&backup), r#"{"model": "sonnet"}"#);
}

/// R51: uninstalling leaves a project as it was before installing, without
/// the folders (.claude/, .agents/ ...) that installing made. Settings that
/// were there before stay, even if they're empty.
#[test]
fn uninstall_leaves_no_empty_folders_behind() {
    let project = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        let out = bin()
            .args(args)
            .args(["--scope", "project", "--yes"])
            .current_dir(project.path())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    let clients = ["claude-code", "codex", "gemini", "cursor"];
    for client in clients {
        run(&["install", client]);
    }
    // .claude, .codex (the hooks) and .agents (its command), .gemini, .cursor.
    assert_eq!(std::fs::read_dir(project.path()).unwrap().count(), 5);
    for client in clients {
        run(&["uninstall", client]);
    }
    let left: Vec<_> = walkdir(project.path());
    assert!(left.is_empty(), "left behind: {left:?}");

    // Settings that were already there are the user's, even empty ones.
    let settings = project.path().join(".claude/settings.json");
    std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
    std::fs::write(&settings, "{}").unwrap();
    run(&["install", "claude-code"]);
    run(&["uninstall", "claude-code"]);
    assert_eq!(read(&settings), "{}\n");
}

/// Every file and folder under `dir`.
fn walkdir(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut all = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        all.push(entry.path());
        if entry.file_type().unwrap().is_dir() {
            all.extend(walkdir(&entry.path()));
        }
    }
    all
}

// ---------- view: a second viewer replaces the first ----------

/// Starts `agent-graph view` on `port` for `home`, and waits for it to print
/// its link. The child, and every line it printed.
fn start_viewer(home: &Path, port: u16) -> (std::process::Child, Vec<String>) {
    use std::io::{BufRead, BufReader};
    let mut child = bin()
        .args(["view", "--port", &port.to_string()])
        .env("AGENT_GRAPH_HOME", home)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut lines = Vec::new();
    for line in BufReader::new(child.stdout.take().unwrap()).lines() {
        let line = line.unwrap();
        let done = line.starts_with("Agent Graph viewer");
        lines.push(line);
        if done {
            break;
        }
    }
    (child, lines)
}

/// A port nothing's listening on.
fn free_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[test]
fn a_new_viewer_replaces_the_one_already_on_its_port() {
    let home = tempfile::tempdir().unwrap();
    let port = free_port();
    let (mut first, lines) = start_viewer(home.path(), port);
    assert!(
        lines.iter().any(|l| l.starts_with("Agent Graph viewer: ")),
        "{lines:?}"
    );
    let first_link = lines.last().unwrap().clone();

    // Another viewer on the port: it stops the first, and serves in its place.
    let (mut second, lines) = start_viewer(home.path(), port);
    assert!(
        lines.iter().any(|l| l.starts_with(&format!(
            "Stopping the viewer already running on port {port} (process {})",
            first.id()
        ))),
        "{lines:?}"
    );
    let second_link = lines.last().unwrap().clone();
    assert!(second_link.starts_with("Agent Graph viewer: "), "{lines:?}");
    assert_ne!(second_link, first_link, "a new viewer, with its own key");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while first.try_wait().unwrap().is_none() {
        assert!(
            std::time::Instant::now() < deadline,
            "the first viewer's still running"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    // The second is the one on the port now.
    assert!(second.try_wait().unwrap().is_none());
    assert!(std::net::TcpListener::bind(("127.0.0.1", port)).is_err());
    second.kill().unwrap();
    second.wait().unwrap();
}

#[test]
fn a_viewer_never_stops_another_program_on_its_port() {
    let home = tempfile::tempdir().unwrap();
    let other = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = other.local_addr().unwrap().port();
    let out = bin()
        .args(["view", "--port", &port.to_string()])
        .env("AGENT_GRAPH_HOME", home.path())
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains(&format!("can't listen on port {port}")),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // Still listening.
    other.set_nonblocking(true).unwrap();
    let _ = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
}

/// Codex on Windows runs each hook as `powershell -NoProfile -Command
/// <command>`, which takes a command starting with a quoted path for a
/// string: the installed command runs there, and records the event.
#[cfg(windows)]
#[test]
fn codex_on_windows_runs_the_installed_hook_through_powershell() {
    let codex = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let out = bin()
        .args(["install", "codex", "--yes"])
        .env("CODEX_HOME", codex.path())
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .current_dir(home.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let hooks: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(codex.path().join("hooks.json")).unwrap())
            .unwrap();
    let command = hooks["hooks"]["SessionStart"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(command.starts_with("& \""), "{command}");

    let payload = r#"{"session_id": "0199aaaa-0000-7000-8000-000000000001", "cwd": "C:/w", "hook_event_name": "SessionStart", "source": "startup"}"#;
    let mut powershell = Command::new("powershell")
        .args(["-NoProfile", "-Command", &command])
        .env("AGENT_GRAPH_HOME", data.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    powershell
        .stdin
        .take()
        .unwrap()
        .write_all(payload.as_bytes())
        .unwrap();
    let out = powershell.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let recorded: Vec<_> = std::fs::read_dir(data.path().join("events"))
        .unwrap()
        .flatten()
        .map(|f| f.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        recorded.iter().any(|f| f.starts_with("codex-")),
        "the session's events: {recorded:?}"
    );
}

/// `install codex` puts the hooks in Codex's hooks file and trusts them in
/// its config.toml, keeping what's there; `uninstall codex` takes both out.
#[test]
fn codex_hooks_are_installed_trusted_and_removed() {
    let codex = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let config = codex.path().join("config.toml");
    std::fs::write(&config, "# Mine.\nmodel = \"gpt-5.5\"\n").unwrap();
    let run = |args: &[&str]| {
        let out = bin()
            .args(args)
            .arg("--yes")
            .env("CODEX_HOME", codex.path())
            .env("HOME", project.path())
            // (Windows' home, for the $agent-graph skill: not the real one.)
            .env("USERPROFILE", project.path())
            .current_dir(project.path())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    let said = run(&["install", "codex"]);
    assert!(said.contains("Trusts Agent Graph's hooks"), "{said}");
    let hooks: serde_json::Value =
        serde_json::from_str(&read(codex.path().join("hooks.json"))).unwrap();
    let start = hooks["hooks"]["SessionStart"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap();
    assert!(start.ends_with("emit --provider codex"), "{start}");
    let trusted = read(&config);
    assert!(
        trusted.starts_with("# Mine.\nmodel = \"gpt-5.5\"\n"),
        "{trusted}"
    );
    // Keyed by the hooks file as Codex names it: on Windows, without the
    // `\\?\` that canonical paths there start with.
    let source = std::fs::canonicalize(codex.path())
        .unwrap()
        .join("hooks.json")
        .display()
        .to_string();
    let source = source.strip_prefix(r"\\?\").unwrap_or(&source);
    let doc: toml_edit::DocumentMut = trusted.parse().unwrap();
    let state = doc["hooks"]["state"].as_table_like().unwrap();
    assert!(
        state.contains_key(&format!("{source}:session_start:0:0")),
        "{trusted}"
    );
    assert_eq!(trusted.matches("trusted_hash").count(), 10);
    assert!(said.contains("Turns on Codex's plan tool"), "{said}");
    assert!(
        trusted.contains("[tools.update_plan]\nenabled = true"),
        "{trusted}"
    );

    // Again: nothing to do.
    assert!(run(&["install", "codex"]).contains("Nothing to do"));

    run(&["uninstall", "codex"]);
    assert_eq!(read(&config), "# Mine.\nmodel = \"gpt-5.5\"\n");
    assert!(!codex.path().join("hooks.json").exists(), "only ever ours");
    assert!(!project.path().join(".agents").exists(), "nor its command");
}

/// Codex's cloud ends a turn on a question (`request_user_input`) and waits
/// for your reply: the turn's Stop leaves the session needing you. A
/// question answered first ends the turn idle, as ever.
#[test]
fn a_codex_turn_that_ends_on_a_question_still_needs_you() {
    let payloads: Vec<String> = std::fs::read_to_string("tests/fixtures/codex/session.jsonl")
        .unwrap()
        .lines()
        .map(String::from)
        .collect();
    let (stop, ask, answer) = (&payloads[17], &payloads[21], &payloads[22]);
    let last_status = |home: &Path| -> serde_json::Value {
        let dir = home.join("events");
        let file = std::fs::read_dir(&dir)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        std::fs::read_to_string(file)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
            .rfind(|e| e["type"] == "status")
            .unwrap()["data"]
            .clone()
    };

    let home = tempfile::tempdir().unwrap();
    for payload in [ask, stop] {
        emit(home.path(), &["--provider", "codex"], payload, &[]);
    }
    let status = last_status(home.path());
    assert_eq!(status["state"], "input_required");
    assert_eq!(status["summary"], "Asks: Keep the cache folder?");

    let home = tempfile::tempdir().unwrap();
    for payload in [ask, answer, stop] {
        emit(home.path(), &["--provider", "codex"], payload, &[]);
    }
    assert_eq!(last_status(home.path())["state"], "idle");
}

/// A Codex cloud task, as it was recorded (`codex/cloud-events.jsonl`): two
/// of its three `spawn_agent` calls failed (a model that doesn't exist), so
/// Codex ran no PostToolUse for them and no agent started. When the turn
/// ends on its question, they're failed agents of their own, and the
/// session still needs you.
#[test]
fn codex_agents_that_never_started_fail_when_the_turn_ends() {
    use agent_graph::event::{Envelope, State};
    let home = tempfile::tempdir().unwrap();
    let session = "01a0f986-0198-7c31-8262-0d8074494407";
    let events = home.path().join("events");
    std::fs::create_dir_all(&events).unwrap();
    let file = events.join(format!("codex-{session}.jsonl"));
    std::fs::copy("tests/fixtures/codex/cloud-events.jsonl", &file).unwrap();
    let stop = serde_json::json!({
        "session_id": session,
        "hook_event_name": "Stop",
        "turn_id": "01a0f986-2000-7000-8000-000000000001",
        "cwd": "/workspace/app",
        "transcript_path": "/opt/codex/sessions/none.jsonl",
    });
    emit(
        home.path(),
        &["--provider", "codex"],
        &stop.to_string(),
        &[],
    );

    let recorded: Vec<Envelope> = std::fs::read_to_string(&file)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let g = common::reduce(recorded);
    let root = format!("codex:{session}");
    assert_eq!(g.nodes[&root].state, State::InputRequired);
    let agents: Vec<_> = g
        .nodes
        .values()
        .filter(|n| n.parent.as_deref() == Some(&root))
        .collect();
    assert_eq!(agents.len(), 3, "Bacon, and the two that never started");
    let failed: Vec<_> = agents.iter().filter(|n| n.state == State::Failed).collect();
    assert_eq!(failed.len(), 2);
    for agent in &failed {
        assert_eq!(agent.purpose.as_deref(), Some("Invalid model hello"));
        assert_eq!(agent.summary.as_deref(), Some("Couldn't start"));
    }
    assert!(g.nodes[&root].spawns.iter().all(|s| s.child.is_some()));

    // A second turn's end changes nothing.
    emit(
        home.path(),
        &["--provider", "codex"],
        &stop.to_string(),
        &[],
    );
    let again = std::fs::read_to_string(&file).unwrap();
    assert_eq!(again.matches("\"spawn.returned\"").count(), 2);
}

/// A Codex agent whose turn errored (a model the account can't use, from a
/// real run: `codex/errored`) gets no SubagentStop, and v2's `wait_agent`
/// doesn't say how it ended. When the session's turn ends, its rollout
/// does: it's failed, with Codex's error.
#[test]
fn a_codex_agent_whose_turn_errored_fails_when_the_turn_ends() {
    use agent_graph::event::{Envelope, State};
    let home = tempfile::tempdir().unwrap();
    let session = "01a0f9a7-fb68-7540-be44-d2999bdc85c8";
    let agent = "01a0f9a8-0e0c-7e12-8a64-156ab6a8ecd7";
    let events = home.path().join("events");
    std::fs::create_dir_all(&events).unwrap();
    let file = events.join(format!("codex-{session}.jsonl"));
    std::fs::copy("tests/fixtures/codex/errored/events.jsonl", &file).unwrap();
    let day = home.path().join("codex/sessions/2026/10/01");
    std::fs::create_dir_all(&day).unwrap();
    let parent = day.join(format!("rollout-2026-10-01T23-48-50-{session}.jsonl"));
    std::fs::write(&parent, "").unwrap();
    std::fs::copy(
        "tests/fixtures/codex/errored/agent-rollout.jsonl",
        day.join(format!("rollout-2026-10-01T23-48-55-{agent}.jsonl")),
    )
    .unwrap();
    let stop = serde_json::json!({
        "session_id": session,
        "hook_event_name": "Stop",
        "turn_id": "01a0f9a7-fbc5-74a0-b932-c953fe261cda",
        "transcript_path": parent,
        "last_assistant_message": "The subagent finished with status errored.",
    });
    emit(
        home.path(),
        &["--provider", "codex"],
        &stop.to_string(),
        &[],
    );

    let recorded: Vec<Envelope> = std::fs::read_to_string(&file)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let g = common::reduce(recorded);
    let node = &g.nodes[&format!("codex:{session}/{agent}")];
    assert_eq!(node.state, State::Failed);
    assert_eq!(
        node.summary.as_deref(),
        Some("The 'no-such-model' model is not supported when using Codex with a ChatGPT account.")
    );
    assert_eq!(g.nodes[&format!("codex:{session}")].state, State::Idle);
}

/// In Codex's cloud, each reply resumes the session on a new machine (from
/// a real task: `codex/cloud-resume-events.jsonl`): an agent the last one
/// left running (sleeping 120 s) went with it, so it's stopped, not working
/// for ever. One Codex brings back starts, and works, again.
#[test]
fn a_resumed_codex_session_stops_the_agents_it_left_running() {
    use agent_graph::event::{Envelope, State};
    let home = tempfile::tempdir().unwrap();
    let session = "01a0f9b6-ce46-7712-a724-05cd298bb0da";
    let events = home.path().join("events");
    std::fs::create_dir_all(&events).unwrap();
    let file = events.join(format!("codex-{session}.jsonl"));
    std::fs::copy("tests/fixtures/codex/cloud-resume-events.jsonl", &file).unwrap();
    let resume = serde_json::json!({
        "session_id": session,
        "hook_event_name": "SessionStart",
        "source": "resume",
        "cwd": "/workspace/app",
        "transcript_path": "/opt/codex/sessions/none.jsonl",
    });
    emit(
        home.path(),
        &["--provider", "codex"],
        &resume.to_string(),
        &[],
    );
    let graph = || {
        let recorded: Vec<Envelope> = std::fs::read_to_string(&file)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        common::reduce(recorded)
    };
    let g = graph();
    let sleeper = g
        .nodes
        .values()
        .find(|n| n.purpose.as_deref() == Some("Sleep 120"))
        .unwrap();
    assert_eq!(sleeper.state, State::Canceled);
    let finished = g
        .nodes
        .values()
        .find(|n| n.purpose.as_deref() == Some("Broken"))
        .unwrap();
    assert_eq!(
        finished.state,
        State::Completed,
        "a finished one is left as it was"
    );

    // Codex brings it back.
    let agent = sleeper.id.rsplit_once('/').unwrap().1.to_string();
    let start = serde_json::json!({
        "session_id": session,
        "agent_id": agent,
        "hook_event_name": "SubagentStart",
        "agent_type": "default",
        "transcript_path": "/opt/codex/sessions/none.jsonl",
    });
    emit(
        home.path(),
        &["--provider", "codex"],
        &start.to_string(),
        &[],
    );
    let prompt = serde_json::json!({
        "session_id": session,
        "agent_id": agent,
        "hook_event_name": "UserPromptSubmit",
    });
    emit(
        home.path(),
        &["--provider", "codex"],
        &prompt.to_string(),
        &[],
    );
    assert_eq!(graph().nodes[&sleeper.id].state, State::Working);
}

/// Cursor runs its CLI's subagents' hooks in conversations of their own,
/// which never start: they're left out, and a chat's are recorded once it
/// has started (docs/cursor.md).
#[test]
fn emit_records_a_cursor_conversation_only_once_it_has_started() {
    let home = tempfile::tempdir().unwrap();
    let payloads = fixture("cursor/session.jsonl");
    let file = home
        .path()
        .join("events/cursor-516781fb-4582-42b3-b4f4-8a294572e5f6.jsonl");
    // A prompt (and so, a status) in a conversation that hasn't started.
    let out = emit(
        home.path(),
        &["--provider", "cursor"],
        &payloads[1].to_string(),
        &[],
    );
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "{}");
    assert!(
        !file.exists(),
        "a conversation that never started isn't recorded"
    );
    // Once it has started, its hooks are.
    for payload in &payloads[..2] {
        emit(
            home.path(),
            &["--provider", "cursor"],
            &payload.to_string(),
            &[],
        );
    }
    let text = read(&file);
    assert!(text.contains("\"type\":\"session.started\""));
    assert!(text.contains("\"type\":\"status\""));
}

// ---------------------------------------------------------------------------
// Cursor's CLI keeps each chat in a database of its own
// (`~/.cursor/chats/<folder's hash>/<chat id>/store.db`): its metadata, as
// hex-encoded JSON, and its todo list, as protobuf records its root names
// (docs/cursor.md). These make one, and run hooks as the CLI would.

const CLI_CHAT: &str = "8a044cae-0d39-4f1d-bbcf-0881cb1d2238";
const CLI_SUBAGENT: &str = "027e8279-0000-4000-8000-000000000000";
const CLI_CALL: &str = "call-e50aa-1\nfc_p3uKWXM-3LYxF7-63a0c753-aws_ue1_0";

/// A protobuf field: length-delimited (`Err`) or a varint (`Ok`).
fn proto(field: u8, value: Result<u64, &[u8]>) -> Vec<u8> {
    let varint = |mut n: u64| {
        let mut out = Vec::new();
        loop {
            let byte = (n & 0x7f) as u8;
            n >>= 7;
            if n == 0 {
                out.push(byte);
                return out;
            }
            out.push(byte | 0x80);
        }
    };
    match value {
        Ok(n) => [vec![field << 3], varint(n)].concat(),
        Err(bytes) => [
            vec![(field << 3) | 2],
            varint(bytes.len() as u64),
            bytes.to_vec(),
        ]
        .concat(),
    }
}

/// A CLI chat's database, in `home`, with `meta` and the todo list `todos`
/// (id, text, status: 1 pending, 2 in progress, 3 completed, 4 cancelled).
fn cli_store(home: &Path, chat: &str, meta: serde_json::Value, todos: &[(&str, &str, u64)]) {
    let dir = home
        .join(".cursor/chats/58f42aa65db0a0223afabff192c472ae")
        .join(chat);
    std::fs::create_dir_all(&dir).unwrap();
    let db = rusqlite::Connection::open(dir.join("store.db")).unwrap();
    db.execute_batch(
        "create table blobs (id text primary key, data blob); \
         create table meta (key text primary key, value text);",
    )
    .unwrap();
    let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let mut root = Vec::new();
    for (i, (id, text, status)) in todos.iter().enumerate() {
        let record = [
            proto(1, Err(id.as_bytes())),
            proto(2, Err(text.as_bytes())),
            proto(3, Ok(*status)),
        ]
        .concat();
        let key = [i as u8 + 1; 32]; // (Cursor's are SHA-256s: any 32 bytes do.)
        db.execute(
            "insert into blobs values (?1, ?2)",
            rusqlite::params![hex(&key), record],
        )
        .unwrap();
        root.extend(proto(3, Err(&key)));
    }
    root.extend(proto(22, Err(b"cli")));
    let root_id = [0xab; 32];
    db.execute(
        "insert into blobs values (?1, ?2)",
        rusqlite::params![hex(&root_id), root],
    )
    .unwrap();
    let mut meta = meta;
    meta["latestRootBlobId"] = hex(&root_id).into();
    db.execute(
        "insert into meta values ('0', ?1)",
        [hex(meta.to_string().as_bytes())],
    )
    .unwrap();
}

/// Runs a Cursor CLI hook with `payload` (its conversation `chat`), from
/// `home`.
fn cli_hook(home: &Path, payload: serde_json::Value) {
    let out = emit(
        &home.join("ag"),
        &["--provider", "cursor"],
        &payload.to_string(),
        &[
            ("HOME", home.to_str().unwrap()),
            ("CURSOR_INVOKED_AS", "agent"),
        ],
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "{}");
}

fn cli_graph(home: &Path) -> String {
    let out = bin()
        .arg("tree")
        .arg("--all")
        .env("AGENT_GRAPH_HOME", home.join("ag"))
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn a_cli_chats_plan_and_todo_list_come_from_its_own_database() {
    let home = tempfile::tempdir().unwrap();
    cli_store(
        home.path(),
        CLI_CHAT,
        serde_json::json!({"agentId": CLI_CHAT, "name": "Changelog Plan", "mode": "plan", "isRunEverything": false}),
        &[
            ("1", "Survey the folder", 3),
            ("2", "Write CHANGELOG.md", 1),
            ("3", "Tag a release", 4),
        ],
    );
    let hook = |name: &str| {
        serde_json::json!({"hook_event_name": name, "conversation_id": CLI_CHAT,
            "status": "completed", "cursor_version": "2026.10.01", "workspace_roots": ["/Users/dev/app"]})
    };
    cli_hook(home.path(), hook("sessionStart"));
    cli_hook(home.path(), hook("stop"));
    let tree = cli_graph(home.path());
    assert!(
        tree.contains("needs you: Plan ready for your review"),
        "{tree}"
    );
    assert!(tree.contains("tasks 2/3"), "{tree}");
}

#[test]
fn a_cli_subagents_hooks_are_the_subagents_in_its_chat() {
    let home = tempfile::tempdir().unwrap();
    cli_store(
        home.path(),
        CLI_CHAT,
        serde_json::json!({"agentId": CLI_CHAT, "mode": "default", "isRunEverything": false}),
        &[],
    );
    cli_store(
        home.path(),
        CLI_SUBAGENT,
        serde_json::json!({"agentId": CLI_SUBAGENT, "mode": "default", "isRunEverything": false,
            "subagentInfo": {"parentAgentId": CLI_CHAT, "rootParentAgentId": CLI_CHAT,
                "toolCallId": CLI_CALL, "typeName": "explore"}}),
        &[],
    );
    let common = |name: &str, conversation: &str| {
        serde_json::json!({"hook_event_name": name, "conversation_id": conversation,
            "cursor_version": "2026.10.01", "workspace_roots": ["/Users/dev/app"]})
    };
    cli_hook(home.path(), common("sessionStart", CLI_CHAT));
    let mut task = common("preToolUse", CLI_CHAT);
    task["tool_name"] = "Task".into();
    task["tool_use_id"] = CLI_CALL.into();
    task["tool_input"] =
        serde_json::json!({"description": "Count the files", "subagent_type": "explore"});
    cli_hook(home.path(), task);
    // The subagent's own hook: its conversation, which says nothing else.
    let mut shell = common("preToolUse", CLI_SUBAGENT);
    shell["tool_name"] = "Shell".into();
    shell["tool_use_id"] = "toolu_1".into();
    shell["tool_input"] = serde_json::json!({"command": "ls src"});
    cli_hook(home.path(), shell);
    // Recorded in its chat's file, not a file of its own.
    let events = home.path().join("ag/events");
    assert!(!events.join(format!("cursor-{CLI_SUBAGENT}.jsonl")).exists());
    let tree = cli_graph(home.path());
    assert!(tree.contains("explore call-e50"), "{tree}");
    assert!(tree.contains("Count the files"), "{tree}");
    // A print run's end is its turn's: the subagent finished with it.
    let mut end = common("sessionEnd", CLI_CHAT);
    end["reason"] = "completed".into();
    cli_hook(home.path(), end);
    let tree = cli_graph(home.path());
    assert!(tree.contains("[completed]  Count the files"), "{tree}");
}
