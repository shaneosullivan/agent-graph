//! Runs the real binary, the way provider hooks and users do.

mod common;

use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use common::fixture;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_agent-graph"))
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
