//! Runs the real binary, the way provider hooks and users do.

mod common;

use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use common::fixture;

/// The binary, without any link to a session this test run is inside.
fn bin() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_agent-graph"));
    for var in ["AGENT_GRAPH_PARENT", "TRACEPARENT", "CLAUDE_ENV_FILE"] {
        command.env_remove(var);
    }
    command
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
        format!("export AGENT_GRAPH_PARENT='{node}'\nexport TRACEPARENT='{traceparent}'\n"),
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
