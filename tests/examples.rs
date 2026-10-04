//! The example logs in `examples/logs`: sessions covering normal work and the
//! edge cases (hung agents, crashed sessions, deadlocks, corrupt files,
//! hostile text, …). This file writes them and checks what the reducer makes
//! of each one.
//!
//! The logs are checked in so they can be browsed and viewed; the test fails
//! if they drift from what's generated here. After changing a scenario or the
//! event format, regenerate them with:
//!
//!     REGENERATE_EXAMPLES=1 cargo test --test examples

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use agent_graph::emit::to_line;
use agent_graph::event::*;
use agent_graph::paths::file_key;
use agent_graph::reducer::{self, Graph};
use agent_graph::store;

/// Every example is written relative to this moment. View them with
/// `AGENT_GRAPH_NOW` set to it (see examples/README.md).
const NOW: &str = "2026-09-25T12:00:00Z";
const STALE_AFTER: Duration = Duration::from_secs(30 * 60);

fn now() -> SystemTime {
    humantime::parse_rfc3339(NOW).unwrap()
}

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/logs/events")
}

// ---------- building logs ----------

/// One events file.
struct Log {
    provider: &'static str,
    session: String,
    text: String,
}

/// Hands out ULIDs that are fixed from run to run and increase in the order
/// events are written.
struct Ids(u128);

impl Log {
    fn new(provider: &'static str, n: u32) -> Log {
        Log {
            provider,
            session: session_id(n),
            text: String::new(),
        }
    }

    fn node(&self) -> String {
        format!("{}:{}", self.provider, self.session)
    }

    fn agent(&self, name: &str) -> String {
        format!("{}/{}", self.node(), agent_id(name))
    }

    /// Adds an event `mins_ago` minutes before `NOW`.
    fn at(
        &mut self,
        ids: &mut Ids,
        mins_ago: f64,
        node: &str,
        parent: Option<&str>,
        payload: Payload,
    ) -> &mut Self {
        let at = now() - Duration::from_millis((mins_ago * 60_000.0) as u64);
        let ms = at
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        ids.0 += 1;
        let event = Envelope {
            v: SCHEMA_VERSION,
            id: ulid::Ulid::from_parts(ms, ids.0).to_string(),
            ts: humantime::format_rfc3339_millis(at).to_string(),
            kind: payload.type_name().to_string(),
            node: node.to_string(),
            parent: parent.map(String::from),
            source: Some(Source {
                provider: self.provider.to_string(),
                provider_version: None,
                adapter: Some("examples".to_string()),
            }),
            trace: None,
            data: payload.to_data(),
        };
        self.text.push_str(&to_line(&event));
        self.text.push('\n');
        self
    }

    /// Adds text exactly as given, e.g. a corrupt line.
    fn raw(&mut self, text: &str) -> &mut Self {
        self.text.push_str(text);
        self
    }

    fn file_name(&self) -> String {
        format!("{}.jsonl", file_key(self.provider, &self.session))
    }
}

fn session_id(n: u32) -> String {
    format!("e{n:07x}-7a9e-4c1d-9b2f-{n:012x}")
}

/// A Claude-style agent id: `a` and 16 hex digits, derived from a name so
/// scenarios can refer to agents by name.
fn agent_id(name: &str) -> String {
    let hash = name.bytes().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100000001b3)
    });
    format!("a{hash:016x}")
}

// Payload shorthands.
fn started(cwd: &str) -> Payload {
    Payload::SessionStarted(SessionStarted {
        cwd: Some(cwd.into()),
        source: Some("startup".into()),
        ..Default::default()
    })
}
fn restarted(source: &str) -> Payload {
    Payload::SessionStarted(SessionStarted {
        source: Some(source.into()),
        ..Default::default()
    })
}
fn ended(reason: &str) -> Payload {
    Payload::SessionEnded(SessionEnded {
        reason: Some(reason.into()),
    })
}
fn status(state: State, summary: Option<&str>) -> Payload {
    Payload::Status(Status {
        state,
        summary: summary.map(String::from),
        title: None,
        turn_end: false,
    })
}
fn working() -> Payload {
    status(State::Working, None)
}
fn idle() -> Payload {
    status(State::Idle, None)
}
fn task(id: &str, text: &str, active: &str) -> Payload {
    Payload::TaskUpserted(TaskUpserted {
        id: id.into(),
        text: Some(text.into()),
        active_text: Some(active.into()),
        status: Some(TaskStatus::Pending),
    })
}
fn task_to(id: &str, status: TaskStatus) -> Payload {
    Payload::TaskUpserted(TaskUpserted {
        id: id.into(),
        text: None,
        active_text: None,
        status: Some(status),
    })
}
fn todos(items: &[(&str, TaskStatus)]) -> Payload {
    Payload::TasksUpdated(TasksUpdated {
        items: items
            .iter()
            .enumerate()
            .map(|(i, (text, status))| TaskItem {
                id: (i + 1).to_string(),
                text: text.to_string(),
                active_text: None,
                status: *status,
            })
            .collect(),
    })
}
fn spawn(call: &str, agent_type: &str, purpose: &str, background: bool) -> Payload {
    Payload::SpawnRequested(SpawnRequested {
        call_id: call.into(),
        kind: SpawnKind::Agent,
        agent_type: Some(agent_type.into()),
        purpose: Some(purpose.into()),
        background,
        run: None,
        title: None,
    })
}
/// Another agent started from the session's shell (design §5.2).
fn shell_spawn(call: &str, program: &str, purpose: &str, background: bool) -> Payload {
    Payload::SpawnRequested(SpawnRequested {
        call_id: call.into(),
        kind: SpawnKind::Session,
        agent_type: Some(program.into()),
        purpose: Some(purpose.into()),
        background,
        run: Some(false),
        title: None,
    })
}
/// A session started by another, which it found through `link`.
fn started_by(cwd: &str, link: &str) -> Payload {
    Payload::SessionStarted(SessionStarted {
        cwd: Some(cwd.into()),
        source: Some("startup".into()),
        link_method: Some(link.into()),
        ..Default::default()
    })
}
/// A session that recorded its agent's process and the ones above it.
fn started_in(cwd: &str, process: &str, ancestors: &[&str]) -> Payload {
    Payload::SessionStarted(SessionStarted {
        cwd: Some(cwd.into()),
        source: Some("startup".into()),
        process: Some(process.into()),
        ancestors: ancestors.iter().map(|a| a.to_string()).collect(),
        ..Default::default()
    })
}
fn returned(call: &str, child: &str, outcome: &str) -> Payload {
    Payload::SpawnReturned(SpawnReturned {
        call_id: call.into(),
        child: Some(child.into()),
        outcome: Some(outcome.into()),
    })
}
fn spawned(agent_type: &str) -> Payload {
    Payload::AgentSpawned(AgentSpawned {
        agent_type: Some(agent_type.into()),
        ..Default::default()
    })
}
fn finished(status: FinishStatus) -> Payload {
    Payload::AgentFinished(AgentFinished {
        status,
        summary: None,
    })
}
fn wait(id: &str, on: &str, reason: &str) -> Payload {
    Payload::WaitStarted(WaitStarted {
        wait_id: id.into(),
        on: on.into(),
        reason: Some(reason.into()),
    })
}
fn message(id: &str, to: &str, summary: &str, reply_to: Option<&str>) -> Payload {
    Payload::MessageSent(MessageSent {
        message_id: id.into(),
        to: to.into(),
        reply_to: reply_to.map(String::from),
        summary: Some(summary.into()),
        body: None,
    })
}

/// Every scenario, keyed by a short name used in the assertions below.
fn scenarios() -> BTreeMap<&'static str, Log> {
    use TaskStatus::*;
    let mut ids = Ids(0);
    let mut out = BTreeMap::new();

    // A normal, healthy session: tasks moving, a finished foreground agent,
    // a background agent still running, and a message to it.
    let mut l = Log::new("claude-code", 1);
    let (s, explore, tests) = (
        l.node(),
        l.agent("healthy-explore"),
        l.agent("healthy-tests"),
    );
    l.at(
        &mut ids,
        25.0,
        &s,
        None,
        started("/home/dev/checkout-service"),
    )
    .at(&mut ids, 24.9, &s, None, working())
    .at(
        &mut ids,
        24.0,
        &s,
        None,
        task(
            "1",
            "Reproduce the double-charge bug",
            "Reproducing the double-charge bug",
        ),
    )
    .at(
        &mut ids,
        23.9,
        &s,
        None,
        task(
            "2",
            "Add an idempotency key to charges",
            "Adding an idempotency key",
        ),
    )
    .at(
        &mut ids,
        23.8,
        &s,
        None,
        task("3", "Write a regression test", "Writing a regression test"),
    )
    .at(&mut ids, 23.0, &s, None, task_to("1", InProgress))
    .at(
        &mut ids,
        22.0,
        &s,
        None,
        spawn(
            "toolu_h1",
            "Explore",
            "Find where payments are retried",
            false,
        ),
    )
    .at(&mut ids, 21.99, &explore, Some(&s), spawned("Explore"))
    .at(
        &mut ids,
        18.0,
        &explore,
        None,
        finished(FinishStatus::Completed),
    )
    .at(
        &mut ids,
        17.99,
        &s,
        None,
        returned("toolu_h1", &explore, "completed"),
    )
    .at(&mut ids, 17.0, &s, None, task_to("1", Completed))
    .at(&mut ids, 16.9, &s, None, task_to("2", InProgress))
    .at(
        &mut ids,
        6.0,
        &s,
        None,
        spawn(
            "toolu_h2",
            "general-purpose",
            "Run the payments test suite",
            true,
        ),
    )
    .at(&mut ids, 5.99, &tests, Some(&s), spawned("general-purpose"))
    .at(
        &mut ids,
        5.98,
        &s,
        None,
        returned("toolu_h2", &tests, "async_launched"),
    )
    .at(
        &mut ids,
        1.0,
        &s,
        None,
        message(
            "msg_h1",
            &agent_id("healthy-tests"),
            "Also run the webhook tests",
            None,
        ),
    );
    out.insert("healthy", l);

    // A foreground agent that stopped reporting 52 minutes ago. The agent is
    // stale; its parent is blocked on it, which is the useful thing to see.
    let mut l = Log::new("claude-code", 2);
    let (s, stuck) = (l.node(), l.agent("hung-explore"));
    l.at(
        &mut ids,
        70.0,
        &s,
        None,
        started("/home/dev/search-indexer"),
    )
    .at(&mut ids, 69.9, &s, None, working())
    .at(
        &mut ids,
        69.0,
        &s,
        None,
        task(
            "1",
            "Rebuild the index schema",
            "Rebuilding the index schema",
        ),
    )
    .at(
        &mut ids,
        68.9,
        &s,
        None,
        task("2", "Backfill documents", "Backfilling documents"),
    )
    .at(&mut ids, 68.0, &s, None, task_to("1", InProgress))
    .at(
        &mut ids,
        52.0,
        &s,
        None,
        spawn("toolu_g1", "Explore", "Profile the slow bulk import", false),
    )
    .at(&mut ids, 51.99, &stuck, Some(&s), spawned("Explore"));
    out.insert("hung-agent", l);

    // A session that was working and then went silent: the process was killed
    // or crashed, so no Stop or SessionEnd ever arrived.
    let mut l = Log::new("claude-code", 3);
    let s = l.node();
    l.at(&mut ids, 190.0, &s, None, started("/home/dev/mobile-app"))
        .at(&mut ids, 189.9, &s, None, working())
        .at(
            &mut ids,
            185.0,
            &s,
            None,
            todos(&[
                ("Upgrade React Native to 0.82", InProgress),
                ("Fix the Android build", Pending),
                ("Re-record the screenshots", Pending),
                ("Update the changelog", Pending),
            ]),
        )
        .at(
            &mut ids,
            150.0,
            &s,
            None,
            todos(&[
                ("Upgrade React Native to 0.82", Completed),
                ("Fix the Android build", InProgress),
                ("Re-record the screenshots", Pending),
                ("Update the changelog", Pending),
            ]),
        );
    out.insert("crashed-session", l);

    // A permission prompt nobody answered for three hours. Waiting on the
    // human isn't a hang, so it's not flagged stale; it's flagged "needs you".
    let mut l = Log::new("claude-code", 4);
    let s = l.node();
    l.at(
        &mut ids,
        200.0,
        &s,
        None,
        started("/home/dev/infra-terraform"),
    )
    .at(&mut ids, 199.9, &s, None, working())
    .at(
        &mut ids,
        199.0,
        &s,
        None,
        task("1", "Plan the VPC changes", "Planning the VPC changes"),
    )
    .at(&mut ids, 198.9, &s, None, task_to("1", InProgress))
    .at(
        &mut ids,
        185.0,
        &s,
        None,
        status(
            State::InputRequired,
            Some("Claude needs your permission to use Bash: terraform apply"),
        ),
    );
    out.insert("needs-you", l);

    // Two sessions each waiting on the other: a deadlock.
    let mut a = Log::new("claude-code", 5);
    let mut b = Log::new("claude-code", 6);
    let (sa, sb) = (a.node(), b.node());
    a.at(&mut ids, 40.0, &sa, None, started("/home/dev/api-gateway"))
        .at(&mut ids, 39.9, &sa, None, working())
        .at(
            &mut ids,
            35.0,
            &sa,
            None,
            wait(
                "w_a",
                &sb,
                "Waiting for the auth service's new token format",
            ),
        );
    b.at(&mut ids, 38.0, &sb, None, started("/home/dev/auth-service"))
        .at(&mut ids, 37.9, &sb, None, working())
        .at(
            &mut ids,
            37.0,
            &sb,
            None,
            task(
                "1",
                "Switch tokens to the new format",
                "Switching tokens to the new format",
            ),
        )
        .at(
            &mut ids,
            34.0,
            &sb,
            None,
            wait("w_b", &sa, "Waiting for the gateway's routes to be updated"),
        );
    out.insert("deadlock-a", a);
    out.insert("deadlock-b", b);

    // A Claude Code session waiting on a Codex session it launched from its
    // shell; Codex linked itself through the environment.
    let mut c = Log::new("claude-code", 7);
    let mut x = Log::new("codex", 8);
    let (sc, sx) = (c.node(), x.node());
    c.at(&mut ids, 20.0, &sc, None, started("/home/dev/web-frontend"))
        .at(&mut ids, 19.9, &sc, None, working())
        .at(
            &mut ids,
            15.0,
            &sc,
            None,
            shell_spawn(
                "toolu_codex",
                "codex",
                "Migrate the design tokens with Codex",
                false,
            ),
        );
    x.at(
        &mut ids,
        14.9,
        &sx,
        Some(&sc),
        started_by("/home/dev/web-frontend", "env"),
    )
    .at(&mut ids, 14.8, &sx, None, working())
    .at(
        &mut ids,
        14.0,
        &sx,
        None,
        todos(&[
            ("Map old tokens to new names", Completed),
            ("Rewrite the CSS variables", InProgress),
            ("Update Storybook", Pending),
        ]),
    )
    .at(&mut ids, 2.0, &sx, None, working());
    out.insert("cross-provider", c);
    out.insert("cross-provider-codex", x);

    // Three agents requested in parallel start in a different order than
    // they were asked for. Each return names its child, which corrects the
    // guesses. One fails; one is still running.
    let mut l = Log::new("claude-code", 9);
    let s = l.node();
    let (ax, ay, az) = (l.agent("par-x"), l.agent("par-y"), l.agent("par-z"));
    l.at(&mut ids, 15.0, &s, None, started("/home/dev/docs-site"))
        .at(&mut ids, 14.95, &s, None, working())
        .at(
            &mut ids,
            14.0,
            &s,
            None,
            spawn("toolu_px", "Explore", "Audit the API reference", false),
        )
        .at(
            &mut ids,
            13.999,
            &s,
            None,
            spawn("toolu_py", "Explore", "Find broken links", false),
        )
        .at(
            &mut ids,
            13.998,
            &s,
            None,
            spawn("toolu_pz", "Explore", "List pages without examples", false),
        )
        .at(&mut ids, 13.99, &az, Some(&s), spawned("Explore"))
        .at(&mut ids, 13.989, &ax, Some(&s), spawned("Explore"))
        .at(&mut ids, 13.988, &ay, Some(&s), spawned("Explore"))
        .at(&mut ids, 9.0, &ax, None, finished(FinishStatus::Completed))
        .at(
            &mut ids,
            8.99,
            &s,
            None,
            returned("toolu_px", &ax, "completed"),
        )
        .at(&mut ids, 7.0, &ay, None, finished(FinishStatus::Failed))
        .at(&mut ids, 6.99, &s, None, returned("toolu_py", &ay, "error"));
    out.insert("parallel", l);

    // Agents three levels deep. The provider only reports the session as a
    // subagent's parent; the spawn requests put each under its real parent.
    let mut l = Log::new("claude-code", 10);
    let s = l.node();
    let (plan, explore, worker) = (
        l.agent("nest-plan"),
        l.agent("nest-explore"),
        l.agent("nest-worker"),
    );
    l.at(&mut ids, 12.0, &s, None, started("/home/dev/ml-pipeline"))
        .at(&mut ids, 11.9, &s, None, working())
        .at(
            &mut ids,
            11.0,
            &s,
            None,
            spawn(
                "toolu_n1",
                "Plan",
                "Plan the feature-store migration",
                false,
            ),
        )
        .at(&mut ids, 10.99, &plan, Some(&s), spawned("Plan"))
        .at(
            &mut ids,
            9.0,
            &plan,
            None,
            spawn(
                "toolu_n2",
                "Explore",
                "Survey the current feature pipelines",
                false,
            ),
        )
        .at(&mut ids, 8.99, &explore, Some(&s), spawned("Explore"))
        .at(
            &mut ids,
            6.0,
            &explore,
            None,
            spawn(
                "toolu_n3",
                "general-purpose",
                "Benchmark the Parquet reader",
                false,
            ),
        )
        .at(
            &mut ids,
            5.99,
            &worker,
            Some(&s),
            spawned("general-purpose"),
        )
        .at(&mut ids, 1.5, &worker, None, working());
    out.insert("nested", l);

    // The session ended while a background agent was still running.
    let mut l = Log::new("claude-code", 11);
    let s = l.node();
    let bg = l.agent("outlived-bg");
    l.at(
        &mut ids,
        30.0,
        &s,
        None,
        started("/home/dev/data-migration"),
    )
    .at(&mut ids, 29.9, &s, None, working())
    .at(
        &mut ids,
        25.0,
        &s,
        None,
        spawn(
            "toolu_o1",
            "general-purpose",
            "Copy the orders table to the new cluster",
            true,
        ),
    )
    .at(&mut ids, 24.99, &bg, Some(&s), spawned("general-purpose"))
    .at(
        &mut ids,
        24.98,
        &s,
        None,
        returned("toolu_o1", &bg, "async_launched"),
    )
    .at(&mut ids, 10.0, &s, None, idle())
    .at(&mut ids, 8.0, &s, None, ended("prompt_input_exit"));
    out.insert("outlived", l);

    // Hooks installed mid-session: the first events come from a subagent,
    // and there's never a session.started. The session is a placeholder.
    let mut l = Log::new("claude-code", 12);
    let s = l.node();
    let orphan = l.agent("orphan-agent");
    l.at(&mut ids, 9.0, &orphan, None, working())
        .at(
            &mut ids,
            8.9,
            &orphan,
            None,
            task(
                "1",
                "Port the invoice PDF renderer",
                "Porting the invoice PDF renderer",
            ),
        )
        .at(
            &mut ids,
            4.0,
            &orphan,
            None,
            finished(FinishStatus::Completed),
        )
        .at(&mut ids, 3.0, &s, None, idle());
    out.insert("orphan", l);

    // Ended yesterday, resumed today, compacted mid-turn (which mustn't
    // reset it), then idle waiting for the next prompt.
    let mut l = Log::new("claude-code", 13);
    let s = l.node();
    l.at(&mut ids, 26.0 * 60.0, &s, None, started("/home/dev/blog"))
        .at(&mut ids, 26.0 * 60.0 - 0.1, &s, None, working())
        .at(&mut ids, 25.0 * 60.0, &s, None, ended("prompt_input_exit"))
        .at(&mut ids, 180.0, &s, None, restarted("resume"))
        .at(&mut ids, 179.0, &s, None, working())
        .at(&mut ids, 150.0, &s, None, restarted("compact"))
        .at(&mut ids, 121.0, &s, None, idle());
    out.insert("resumed", l);

    // Text that tries to break the viewer and the image: markup, SVG
    // injection, emoji, right-to-left and CJK text, newlines, a very long
    // line, and an odd folder name.
    let mut l = Log::new("claude-code", 14);
    let s = l.node();
    let evil = l.agent("hostile-agent");
    let long = "Refactor the settings loader so that ".repeat(8) + "it finally works";
    l.at(
        &mut ids,
        10.0,
        &s,
        None,
        started("/home/dev/ünïcödé-项目 & <b>"),
    )
    .at(&mut ids, 9.9, &s, None, working())
    .at(
        &mut ids,
        9.0,
        &s,
        None,
        task(
            "1",
            "<img src=x onerror=alert('xss')>",
            "<script>alert(1)</script>",
        ),
    )
    .at(
        &mut ids,
        8.9,
        &s,
        None,
        task(
            "2",
            "Ship 🚀 the \"quoted\" & <escaped> release",
            "Shipping 🚀",
        ),
    )
    .at(
        &mut ids,
        8.8,
        &s,
        None,
        task("3", "مراجعة الكود قبل الدمج", "مراجعة الكود"),
    )
    .at(
        &mut ids,
        8.7,
        &s,
        None,
        task("4", "修复登录错误并添加测试用例", "修复登录错误"),
    )
    .at(&mut ids, 8.6, &s, None, task("5", &long, &long))
    .at(
        &mut ids,
        8.55,
        &s,
        None,
        task(
            "6",
            "Terminal \u{1b}[2J\u{1b}[31mescape\u{1b}[0m and \u{202e}desrever",
            "Escaping",
        ),
    )
    .at(&mut ids, 8.5, &s, None, task_to("2", InProgress))
    .at(
        &mut ids,
        8.0,
        &s,
        None,
        spawn(
            "toolu_x1",
            "Explore",
            "</text><script>alert(1)</script>\nline two",
            false,
        ),
    )
    .at(&mut ids, 7.99, &evil, Some(&s), spawned("Explore"))
    .at(
        &mut ids,
        7.0,
        &s,
        None,
        message(
            "msg_x1",
            &agent_id("hostile-agent"),
            "**bold** [link](javascript:alert(1))",
            None,
        ),
    )
    .at(
        &mut ids,
        6.0,
        &evil,
        None,
        status(
            State::InputRequired,
            Some("Needs 🔑 access to \"prod\" <db>"),
        ),
    );
    out.insert("hostile-text", l);

    // Thirty tasks: the image lists twelve and counts the rest.
    let mut l = Log::new("claude-code", 15);
    let s = l.node();
    let items: Vec<(String, TaskStatus)> = (1..=30)
        .map(|i| {
            let status = if i <= 12 {
                Completed
            } else if i == 13 {
                InProgress
            } else {
                Pending
            };
            (
                format!("Migrate package {i:02} to the new build system"),
                status,
            )
        })
        .collect();
    let items: Vec<(&str, TaskStatus)> = items.iter().map(|(t, s)| (t.as_str(), *s)).collect();
    l.at(&mut ids, 5.0, &s, None, started("/home/dev/monorepo"))
        .at(&mut ids, 4.9, &s, None, working())
        .at(&mut ids, 1.0, &s, None, todos(&items));
    out.insert("many-tasks", l);

    // Idle for three days: hidden from "last 24 hours" views, never stale.
    let mut l = Log::new("claude-code", 16);
    let s = l.node();
    l.at(
        &mut ids,
        3.0 * 24.0 * 60.0,
        &s,
        None,
        started("/home/dev/old-experiment"),
    )
    .at(&mut ids, 3.0 * 24.0 * 60.0 - 1.0, &s, None, working())
    .at(&mut ids, 3.0 * 24.0 * 60.0 - 20.0, &s, None, idle());
    out.insert("idle-old", l);

    // A file damaged by a crash: a garbage line, a blank line, and a last
    // line cut off mid-write. The good lines still count.
    let mut l = Log::new("claude-code", 17);
    let s = l.node();
    l.at(&mut ids, 45.0, &s, None, started("/home/dev/crash-test"))
        .at(&mut ids, 44.9, &s, None, working())
        .raw("this line is not JSON at all\n")
        .raw("\n")
        .at(
            &mut ids,
            44.0,
            &s,
            None,
            task("1", "Stress-test the queue", "Stress-testing the queue"),
        )
        .at(&mut ids, 43.0, &s, None, task_to("1", InProgress))
        .raw(r#"{"v":1,"id":"01K5ZZZZZZZZZZZZZZZZZZZZZZ","ts":"2026-09-25T11:17:"#);
    out.insert("corrupt-file", l);

    // Events a newer writer might send: an unknown type, a known type with
    // data we can't read, and fields we don't know. All ignored, not fatal.
    let mut l = Log::new("claude-code", 18);
    let s = l.node();
    l.at(&mut ids, 3.0, &s, None, started("/home/dev/schema-v2"))
        .at(&mut ids, 2.9, &s, None, working())
        .at(
            &mut ids,
            2.8,
            &s,
            None,
            Payload::Unknown(serde_json::json!({"hook_event_name": "PreCompact"})),
        );
    let unknown_type = r#"{"v":1,"id":"01K600000000000000000000FT","ts":"2026-09-25T11:57:30.000Z","type":"future.thing","node":"claude-code:e0000012-7a9e-4c1d-9b2f-000000000012","data":{"anything":[1,2,3]}}"#;
    let bad_state = r#"{"v":1,"id":"01K600000000000000000000BS","ts":"2026-09-25T11:57:40.000Z","type":"status","node":"claude-code:e0000012-7a9e-4c1d-9b2f-000000000012","data":{"state":"exploded"}}"#;
    let extra_fields = r#"{"v":1,"id":"01K600000000000000000000XF","ts":"2026-09-25T11:57:50.000Z","type":"status","node":"claude-code:e0000012-7a9e-4c1d-9b2f-000000000012","data":{"state":"working","summary":"Trying the v2 format","mood":"optimistic"},"future_envelope_field":true}"#;
    l.raw(&format!("{unknown_type}\n{bad_state}\n{extra_fields}\n"));
    out.insert("future-proof", l);

    // `agent-graph run --name nightly-docs -- ./update-docs.sh`: a script
    // that starts two headless Claude Code sessions, grouped under the run.
    // One has finished; the other is still going.
    let mut r = Log::new("run", 0x13);
    let mut w1 = Log::new("claude-code", 0x14);
    let mut w2 = Log::new("claude-code", 0x15);
    let (sr, s1, s2) = (r.node(), w1.node(), w2.node());
    r.at(
        &mut ids,
        25.0,
        &sr,
        None,
        Payload::SessionStarted(SessionStarted {
            cwd: Some("/home/dev/docs".into()),
            source: Some("run".into()),
            title: Some("nightly-docs".into()),
            ..Default::default()
        }),
    )
    .at(&mut ids, 25.0, &sr, None, working());
    w1.at(
        &mut ids,
        24.9,
        &s1,
        Some(&sr),
        started_by("/home/dev/docs/api", "run"),
    )
    .at(&mut ids, 24.8, &s1, None, working())
    .at(
        &mut ids,
        24.0,
        &s1,
        None,
        todos(&[
            ("Regenerate the API reference", Completed),
            ("Check the examples compile", Completed),
        ]),
    )
    .at(&mut ids, 11.0, &s1, None, ended("other"));
    w2.at(
        &mut ids,
        24.9,
        &s2,
        Some(&sr),
        started_by("/home/dev/docs/guides", "run"),
    )
    .at(&mut ids, 24.8, &s2, None, working())
    .at(
        &mut ids,
        23.0,
        &s2,
        None,
        todos(&[
            ("Update the install guide", Completed),
            ("Rewrite the quick start", InProgress),
            ("Fix broken links", Pending),
        ]),
    )
    .at(&mut ids, 1.0, &s2, None, working());
    out.insert("run-workers", r);
    out.insert("run-workers-api", w1);
    out.insert("run-workers-guides", w2);

    // A session started from another's shell that didn't inherit its
    // identity (say, hooks were installed after the parent started): linked
    // because the parent's agent process is among its own ancestors.
    let mut p = Log::new("claude-code", 0x16);
    let mut k = Log::new("claude-code", 0x17);
    let (sp, sk) = (p.node(), k.node());
    p.at(
        &mut ids,
        40.0,
        &sp,
        None,
        started_in(
            "/home/dev/billing",
            "41000@1790380000000000",
            &["40990@1790370000000000"],
        ),
    )
    .at(&mut ids, 39.9, &sp, None, working())
    .at(
        &mut ids,
        31.0,
        &sp,
        None,
        shell_spawn(
            "toolu_review",
            "claude",
            "Get a second opinion on the refund logic",
            false,
        ),
    );
    k.at(
        &mut ids,
        30.9,
        &sk,
        None,
        started_in(
            "/home/dev/billing",
            "41500@1790381000000000",
            &[
                "41490@1790380900000000",
                "41000@1790380000000000",
                "40990@1790370000000000",
            ],
        ),
    )
    .at(&mut ids, 30.8, &sk, None, working())
    .at(&mut ids, 3.0, &sk, None, working());
    out.insert("process-link", p);
    out.insert("process-link-review", k);

    out
}

/// Files that aren't sessions: the loader must skip or survive them.
fn other_files() -> Vec<(&'static str, &'static str)> {
    vec![
        ("empty.jsonl", ""),
        (
            "notes.txt",
            "Not an events file; the loader ignores anything that isn't .jsonl.\n",
        ),
    ]
}

// ---------- the golden files ----------

#[test]
fn checked_in_logs_match_the_generator() {
    let dir = dir();
    let mut expected: BTreeMap<String, String> = scenarios()
        .into_values()
        .map(|log| (log.file_name(), log.text))
        .collect();
    expected.extend(
        other_files()
            .into_iter()
            .map(|(n, t)| (n.to_string(), t.to_string())),
    );

    if std::env::var_os("REGENERATE_EXAMPLES").is_some() {
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (name, text) in &expected {
            std::fs::write(dir.join(name), text).unwrap();
        }
    }

    let mut actual = BTreeMap::new();
    for entry in
        std::fs::read_dir(&dir).expect("examples/logs/events exists; see the top of this file")
    {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        actual.insert(
            name,
            std::fs::read_to_string(&path)
                .unwrap()
                .replace("\r\n", "\n"),
        );
    }
    assert_eq!(
        actual.keys().collect::<Vec<_>>(),
        expected.keys().collect::<Vec<_>>(),
        "example files differ; regenerate with REGENERATE_EXAMPLES=1 cargo test --test examples"
    );
    for (name, text) in &expected {
        assert!(
            &actual[name] == text,
            "{name} differs from the generator; regenerate with REGENERATE_EXAMPLES=1 cargo test --test examples"
        );
    }
}

// ---------- what the reducer makes of them ----------

fn graph() -> (Graph, usize) {
    let loaded = store::load_events(&dir()).unwrap();
    let graph = reducer::reduce(
        loaded.events,
        &reducer::Options {
            now: now(),
            stale_after: STALE_AFTER,
        },
    );
    (graph, loaded.skipped_lines)
}

fn session(name: &str) -> String {
    let logs = scenarios();
    logs[name].node()
}

fn agent(scenario: &str, name: &str) -> String {
    scenarios()[scenario].agent(name)
}

#[test]
fn every_session_is_a_root_and_corrupt_lines_are_skipped() {
    let (g, skipped) = graph();
    // A garbage line and a cut-off line (the blank line isn't counted).
    assert_eq!(skipped, 2);
    // Every scenario is its own session, except the ones started by another:
    // the Codex session, the two run workers, and the process-linked review.
    assert_eq!(g.roots.len(), scenarios().len() - 4);
}

#[test]
fn healthy_session_is_working_and_nothing_is_stale() {
    let (g, _) = graph();
    let s = &g.nodes[&session("healthy")];
    assert_eq!(s.state, State::Working);
    assert!(
        !s.stale && s.blocked.is_none(),
        "a background agent doesn't block"
    );
    assert_eq!((s.tasks.len(), s.open_tasks), (3, 2));
    let tests = &g.nodes[&agent("healthy", "healthy-tests")];
    assert_eq!(tests.background, Some(true));
    assert!(!tests.stale);
    assert_eq!(tests.messages.len(), 1, "received the message");
}

#[test]
fn hung_agent_is_stale_and_its_parent_is_waiting_on_it() {
    let (g, _) = graph();
    let stuck = agent("hung-agent", "hung-explore");
    assert!(g.nodes[&stuck].stale);
    let s = &g.nodes[&session("hung-agent")];
    assert!(!s.stale, "the parent is waiting, not hung");
    assert_eq!(s.blocked.as_ref().unwrap().on, vec![stuck]);
}

#[test]
fn crashed_session_is_stale() {
    let (g, _) = graph();
    let s = &g.nodes[&session("crashed-session")];
    assert_eq!(s.state, State::Working);
    assert!(s.stale);
    assert_eq!(
        s.headline.as_deref(),
        Some("Fix the Android build (+2 pending)")
    );
}

#[test]
fn unanswered_prompt_needs_you_but_is_not_stale() {
    let (g, _) = graph();
    let s = &g.nodes[&session("needs-you")];
    assert_eq!(s.state, State::InputRequired);
    assert!(!s.stale);
    assert!(s.attention.as_deref().unwrap().contains("terraform apply"));
}

#[test]
fn deadlock_is_detected_on_both_sides() {
    let (g, _) = graph();
    for name in ["deadlock-a", "deadlock-b"] {
        let blocked = g.nodes[&session(name)].blocked.as_ref().unwrap();
        assert!(blocked.cycle, "{name}");
        assert!(!g.nodes[&session(name)].stale, "{name}");
    }
    let tree = agent_graph::render::tree(&g, &[session("deadlock-a")]);
    assert!(tree.contains("DEADLOCK"), "{tree}");
}

#[test]
fn a_run_groups_the_sessions_its_script_started() {
    let (g, _) = graph();
    let run = session("run-workers");
    let (api, guides) = (session("run-workers-api"), session("run-workers-guides"));
    assert_eq!(g.nodes[&run].children, [api.clone(), guides.clone()]);
    assert_eq!(g.nodes[&run].title.as_deref(), Some("nightly-docs"));
    assert_eq!(g.nodes[&api].link.as_deref(), Some("run"));
    assert_eq!(g.nodes[&api].state, State::Completed);
    assert_eq!(g.nodes[&guides].state, State::Working);
    assert!(g.roots.contains(&run));
}

#[test]
fn a_session_is_linked_by_process_and_waited_on() {
    let (g, _) = graph();
    let (parent, review) = (session("process-link"), session("process-link-review"));
    assert_eq!(g.nodes[&review].parent.as_deref(), Some(parent.as_str()));
    assert_eq!(g.nodes[&review].link.as_deref(), Some("process"));
    assert_eq!(g.nodes[&review].spawned_by.as_deref(), Some("toolu_review"));
    assert_eq!(g.nodes[&parent].blocked.as_ref().unwrap().on, [review]);
}

#[test]
fn cross_provider_wait_counts_the_other_sessions_work() {
    let (g, _) = graph();
    let (claude, codex) = (session("cross-provider"), session("cross-provider-codex"));
    assert_eq!(g.nodes[&codex].parent.as_deref(), Some(claude.as_str()));
    assert_eq!(g.nodes[&codex].provider, "codex");
    assert_eq!(g.nodes[&codex].spawned_by.as_deref(), Some("toolu_codex"));
    let blocked = g.nodes[&claude].blocked.as_ref().unwrap();
    assert_eq!(blocked.on, vec![codex.clone()]);
    assert_eq!((blocked.nodes, blocked.open_tasks), (1, 2));
    assert!(!blocked.cycle);
}

#[test]
fn parallel_agents_end_up_with_the_right_requests() {
    let (g, _) = graph();
    let s = &g.nodes[&session("parallel")];
    let pairs: Vec<(&str, Option<&str>)> = s
        .spawns
        .iter()
        .map(|sp| (sp.call_id.as_str(), sp.child.as_deref()))
        .collect();
    let (ax, ay, az) = (
        agent("parallel", "par-x"),
        agent("parallel", "par-y"),
        agent("parallel", "par-z"),
    );
    assert_eq!(
        pairs,
        [
            ("toolu_px", Some(ax.as_str())),
            ("toolu_py", Some(ay.as_str())),
            ("toolu_pz", Some(az.as_str()))
        ]
    );
    assert_eq!(
        g.nodes[&ax].purpose.as_deref(),
        Some("Audit the API reference")
    );
    assert_eq!(g.nodes[&ay].state, State::Failed);
    assert_eq!(
        g.nodes[&az].purpose.as_deref(),
        Some("List pages without examples")
    );
    // Still waiting on the one that hasn't returned.
    assert_eq!(s.blocked.as_ref().unwrap().on, vec![az]);
}

#[test]
fn nested_agents_sit_under_the_agent_that_asked_for_them() {
    let (g, _) = graph();
    let (plan, explore, worker) = (
        agent("nested", "nest-plan"),
        agent("nested", "nest-explore"),
        agent("nested", "nest-worker"),
    );
    assert_eq!(
        g.nodes[&plan].parent.as_deref(),
        Some(session("nested").as_str())
    );
    assert_eq!(g.nodes[&explore].parent.as_deref(), Some(plan.as_str()));
    assert_eq!(g.nodes[&worker].parent.as_deref(), Some(explore.as_str()));
    // Work remaining before the session can continue: the whole chain.
    assert_eq!(
        g.nodes[&session("nested")].blocked.as_ref().unwrap().nodes,
        3
    );
}

#[test]
fn background_agent_is_canceled_when_its_session_ends() {
    let (g, _) = graph();
    assert_eq!(g.nodes[&session("outlived")].state, State::Completed);
    assert_eq!(
        g.nodes[&agent("outlived", "outlived-bg")].state,
        State::Canceled
    );
}

#[test]
fn orphan_agent_gets_a_placeholder_session() {
    let (g, _) = graph();
    let s = &g.nodes[&session("orphan")];
    assert_eq!(s.cwd, None);
    assert_eq!(s.state, State::Idle);
    assert_eq!(s.children, vec![agent("orphan", "orphan-agent")]);
}

#[test]
fn resumed_session_is_idle_and_compaction_did_not_reset_it() {
    let (g, _) = graph();
    let s = &g.nodes[&session("resumed")];
    assert_eq!(s.state, State::Idle);
    assert_eq!(s.ended_at, None);
    assert!(!s.stale);
}

#[test]
fn many_tasks_are_all_kept() {
    let (g, _) = graph();
    let s = &g.nodes[&session("many-tasks")];
    assert_eq!((s.tasks.len(), s.open_tasks), (30, 18));
}

#[test]
fn old_idle_session_is_not_stale() {
    let (g, _) = graph();
    assert!(!g.nodes[&session("idle-old")].stale);
}

#[test]
fn corrupt_file_keeps_its_good_lines() {
    let (g, _) = graph();
    let s = &g.nodes[&session("corrupt-file")];
    assert_eq!(s.tasks.len(), 1);
    assert!(s.stale, "it crashed while working");
}

#[test]
fn events_from_the_future_are_ignored() {
    let (g, _) = graph();
    let s = &g.nodes[&session("future-proof")];
    assert_eq!(s.state, State::Working, "the bad status was ignored");
    assert_eq!(s.summary.as_deref(), Some("Trying the v2 format"));
}

#[test]
fn every_session_renders_and_hostile_text_is_escaped() {
    let (g, _) = graph();
    for root in &g.roots {
        let svg = agent_graph::image::svg(
            &g,
            std::slice::from_ref(root),
            &agent_graph::image::Options {
                theme: agent_graph::image::Theme::Light,
                as_of: now(),
            },
        );
        assert!(
            !svg.contains("<script") && !svg.contains("<img") && !svg.contains("<b>"),
            "{root}"
        );
        agent_graph::image::png(&svg).unwrap_or_else(|e| panic!("{root}: {e}"));
    }
    let svg = agent_graph::image::svg(
        &g,
        &[session("hostile-text")],
        &agent_graph::image::Options {
            theme: agent_graph::image::Theme::Dark,
            as_of: now(),
        },
    );
    assert!(svg.contains("&lt;img src=x onerror=alert(&apos;xss&apos;)&gt;"));
    assert!(svg.contains("ünïcödé-项目 &amp; &lt;b&gt;"));
    assert!(!svg.contains('\u{202e}') && !svg.contains('\u{1b}'));
}

#[test]
fn examples_follow_the_schema_except_where_they_break_it_on_purpose() {
    let schema: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("schema/event.schema.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let on_purpose = [
        scenarios()["future-proof"].file_name(),
        scenarios()["corrupt-file"].file_name(),
    ];
    for (name, log) in scenarios().into_values().map(|l| (l.file_name(), l)) {
        if on_purpose.contains(&name) {
            continue;
        }
        for line in log.text.lines() {
            let value: serde_json::Value = serde_json::from_str(line).unwrap();
            assert!(validator.is_valid(&value), "{name}: {line}");
        }
    }
}

#[test]
fn terminal_output_cannot_be_hijacked_by_model_text() {
    let (g, _) = graph();
    let root = session("hostile-text");
    let text = agent_graph::render::tree(&g, std::slice::from_ref(&root));
    assert!(
        !text.contains('\u{1b}'),
        "no escape sequences reach the terminal"
    );
    assert!(!text.contains('\u{202e}'), "no bidi overrides");
    // One line per node: newlines inside model text don't break the layout.
    let nodes = 1 + g.nodes[&root].children.len();
    assert_eq!(text.lines().count(), nodes, "{text}");
}
