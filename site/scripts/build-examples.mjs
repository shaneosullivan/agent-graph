// Builds the examples on the home page (app/examples.tsx): each a log of
// its own in public/examples/<slug>.jsonl, which /examples/<slug> opens in
// the viewer, and lib/examples.json, which lists them. Most are sessions
// from ../examples/logs (see its README), put together to show one thing
// each; "large" and "team" are made up here (`madeUp`), to show a big
// graph and many sessions at once. The output's committed,
// so building the site never needs this; run it after changing them:
//
//   npm run build-examples

import {mkdirSync, readFileSync, readdirSync, writeFileSync} from "node:fs";
import {dirname, join} from "node:path";
import {fileURLToPath} from "node:url";

const site = join(dirname(fileURLToPath(import.meta.url)), "..");
const events = join(site, "../examples/logs/events");
const out = join(site, "public/examples");

/** Every example: what it shows, and the example sessions (by number) or generator it's made of. */
const EXAMPLES = [
  {
    slug: "needs-you",
    title: "Waiting for you",
    description:
      "A session stopped at a permission prompt, beside one that's working: it's flagged, counted at the top of the list, and pulses until you answer.",
    tags: ["Needs you"],
    sessions: ["04", "01"],
    open: "04",
    view: "graph",
  },
  {
    slug: "tasks",
    title: "Tasks, done and to do",
    description:
      "One session 12 tasks into 30, another checking them off as it goes: counts on every card, and the whole list in its details.",
    tags: ["Tasks", "Progress"],
    sessions: ["0f", "01"],
    open: "0f",
    view: "cards",
  },
  {
    slug: "parallel",
    title: "Agents in parallel",
    description:
      "Three agents started at once: one finished, one failed, one still running, and the session waiting only on that one.",
    tags: ["Running", "Completed", "Failed"],
    sessions: ["09"],
    open: "09",
    view: "graph",
  },
  {
    slug: "nested",
    title: "Agents within agents",
    description:
      "A plan that started an explorer, which started a worker: each three levels down, under the agent that asked for it.",
    tags: ["Nesting"],
    sessions: ["0a"],
    open: "0a",
    view: "graph",
  },
  {
    slug: "deadlock",
    title: "A deadlock",
    description:
      "Two sessions each waiting on the other: both marked, with the wait drawn between them.",
    tags: ["Waits", "Deadlock"],
    sessions: ["05", "06"],
    open: "05",
    view: "graph",
  },
  {
    slug: "scripts",
    title: "Sessions a script started",
    description:
      "A command run with agent-graph run that started two headless Claude Code sessions, both under it; and a session another one started from its shell.",
    tags: ["agent-graph run", "Linked sessions"],
    sessions: ["13", "14", "15", "16", "17"],
    open: "13",
    view: "graph",
  },
  {
    slug: "large",
    title: "A large graph",
    description:
      "A migration run by six planners, each with explorers and workers under them: 61 sessions and agents, 40 tasks, some done, some running, one failed, one waiting for you.",
    tags: ["Scale", "Graph view"],
    generate: () => madeUp([LARGE]),
    open: {madeUp: 0},
    view: "graph",
  },
  {
    slug: "team",
    title: "A busy team",
    description:
      "A dozen sessions open at once, of every size: the large migration, busy ones, small ones, a Codex session, two waiting for you. Zoom out in the graph to see them all round the one you're on.",
    tags: ["Many sessions", "Zoomed out", "Needs you"],
    generate: () => madeUp(TEAM),
    open: {madeUp: 0},
    view: "graph",
    zoomOut: true,
  },
  {
    slug: "everything",
    title: "A busy day",
    description:
      "Every example at once, about thirty-five sessions: the list by folder, deadlocks and scripts, and everything the busy team was doing.",
    tags: ["Sessions list", "Every kind"],
    sessions: "all",
    open: {madeUp: 0},
    view: "graph",
    zoomOut: true,
  },
];

/** The example session files, by their number (the last two digits of `e00000NN`). */
function sessionFiles() {
  const files = new Map();
  for (const name of readdirSync(events)) {
    const m = name.match(/-e000000?([0-9a-f]{2})-[^.]*\.jsonl$/);
    if (m) files.set(m[1], join(events, name));
  }
  return files;
}

/** An example session's id, from its file's name (`<provider>-<id>.jsonl`). */
function fileSessionId(file) {
  const [, provider, rest] = file.match(
    /([a-z-]+)-(e[0-9a-f]{7}-[0-9a-f-]+)\.jsonl$/,
  );
  return `${provider}:${rest}`;
}

// ---------- made-up sessions ----------

/** A made-up session's id (see `madeUp`). */
function madeUpId(spec) {
  const hex = (0x100 + spec.num).toString(16);
  return `${spec.provider || "claude-code"}:e0000${hex}-7a9e-4c1d-9b2f-000000000${hex}`;
}

/**
 * Writes the events for made-up sessions, from short descriptions of them:
 *
 *   {num, cwd, provider?, at?, state?, summary?, tasks?: [total, done],
 *    agents?: [{type, purpose, state?, summary?, agents?: [...]}, ...]}
 *
 * `num` names the session (e00001NN); `at` is how many minutes before
 * 12:00 it started; a session's `state` (working, idle, input_required,
 * completed) and each agent's (working, completed, failed, input_required)
 * are how they are at the end. Events are a few seconds apart, in order,
 * so each agent answers the request just before it.
 */
function madeUp(specs) {
  const lines = [];
  let n = 0;
  for (const spec of specs) {
    const provider = spec.provider || "claude-code";
    const session = madeUpId(spec);
    let at = Date.parse("2026-09-25T12:00:00.000Z") - (spec.at ?? 40) * 60000;
    const ev = (node, type, data, extra = {}) => {
      n++;
      at += 3000 + ((n * 7919) % 6000); // a few seconds apart, not evenly
      lines.push(
        JSON.stringify({
          v: 1,
          id: `01M3L${n.toString(36).toUpperCase().padStart(21, "0")}`,
          ts: new Date(at).toISOString(),
          type,
          node,
          ...extra,
          source: {provider, adapter: "examples"},
          data,
        }),
      );
    };
    ev(session, "session.started", {cwd: spec.cwd, source: "startup"});
    ev(session, "status", {state: "working"});
    if (spec.tasks) {
      const [total, done] = spec.tasks;
      ev(session, "tasks.updated", {
        items: Array.from({length: total}, (_, i) => ({
          id: String(i + 1),
          text: `${spec.taskText || "Step"} ${i + 1}`,
          status:
            i < done ? "completed" : i === done ? "in_progress" : "pending",
        })),
      });
    }
    let calls = 0;
    const started = [];
    // Each agent started under the one that asked for it (Claude Code's
    // agents all name the session as their parent; the request each answers
    // puts it under the agent that made it), depth first.
    const start = (parent, a) => {
      calls++;
      const call = `toolu_${spec.num}_${calls}`;
      // Named by the first eight characters of its id: the session's number
      // and its own come first, so each reads differently.
      const id = `${session}/a${spec.num.toString(16).padStart(2, "0")}${calls.toString(16).padStart(5, "0")}${"0".repeat(9)}`;
      ev(parent, "spawn.requested", {
        call_id: call,
        kind: "agent",
        agent_type: a.type,
        purpose: a.purpose,
        background: false,
      });
      ev(id, "agent.spawned", {agent_type: a.type}, {parent: session});
      ev(id, "status", {state: "working"});
      const agent = {id, call, parent, spec: a};
      started.push(agent);
      for (const child of a.agents || []) start(id, child);
    };
    for (const a of spec.agents || []) start(session, a);
    // Then how each ended up: the deepest first, so parents hear back last.
    for (const agent of [...started].reverse()) {
      const state = agent.spec.state || "working";
      if (state === "completed" || state === "failed") {
        ev(agent.id, "agent.finished", {status: state});
        ev(agent.parent, "spawn.returned", {
          call_id: agent.call,
          child: agent.id,
          outcome:
            agent.spec.summary || (state === "failed" ? "Failed" : "Done"),
        });
      } else if (state === "input_required") {
        ev(agent.id, "status", {
          state: "input_required",
          summary: agent.spec.summary || "Claude needs your permission",
        });
      }
    }
    const state = spec.state || "working";
    if (state === "completed") {
      ev(session, "status", {state: "idle"});
      ev(session, "session.ended", {reason: "prompt_input_exit"});
    } else if (state !== "working") {
      ev(session, "status", {
        state,
        ...(spec.summary ? {summary: spec.summary} : {}),
      });
    }
  }
  return lines.join("\n") + "\n";
}

/** `count` agents of `type`, each doing `what` (numbered), in the states given in turn. */
function agents(count, type, what, states = ["working"]) {
  return Array.from({length: count}, (_, i) => ({
    type,
    purpose: `${what} ${i + 1}`,
    state: states[i % states.length],
  }));
}

/**
 * A migration run by six planners, each with three explorers, each with
 * two workers: 61 sessions and agents, 40 tasks. The first planners' trees
 * are done, a worker under the third failed, the rest are at work, and one
 * worker is waiting for you.
 */
const LARGE = {
  num: 0,
  cwd: "/home/dev/platform-rewrite",
  at: 50,
  tasks: [40, 24],
  taskText: "Move a service to the new platform: part",
  agents: [
    "billing",
    "accounts",
    "search",
    "notifications",
    "reporting",
    "storage",
  ].map((area, p) => ({
    type: "Plan",
    purpose: `Plan the ${area} migration`,
    state: p < 2 ? "completed" : "working",
    agents: ["data", "API", "jobs"].map((part, e) => ({
      type: "Explore",
      purpose: `Map the ${area} ${part} to move`,
      state: p < 2 || (p === 2 && e !== 1) ? "completed" : "working",
      agents: [1, 2].map(k => ({
        type: "general-purpose",
        purpose: `Port ${area} ${part}, part ${k}`,
        ...(p < 2 || (p === 2 && !(e === 1 && k === 1))
          ? {state: "completed", summary: "Ported, tests passing"}
          : p === 2
            ? {state: "failed", summary: "The schema change broke replication"}
            : p === 3 && e === 0 && k === 1
              ? {
                  state: "input_required",
                  summary:
                    "Claude needs your permission to run the database migration",
                }
              : {state: (p + e + k) % 3 === 0 ? "completed" : "working"}),
      })),
    })),
  })),
};

/** A team's morning: a dozen sessions, of every size and kind, at every stage. */
const TEAM = [
  LARGE,
  {
    num: 1,
    cwd: "/home/dev/checkout",
    at: 35,
    tasks: [9, 5],
    taskText: "Checkout fix",
    agents: agents(3, "Explore", "Trace the payment path, part", [
      "completed",
      "working",
      "working",
    ]),
  },
  {
    num: 2,
    cwd: "/home/dev/mobile-app",
    at: 28,
    state: "input_required",
    summary:
      "Claude needs your permission to use Bash: xcodebuild -scheme Release",
    tasks: [6, 2],
    taskText: "Release build step",
    agents: [
      {
        type: "Explore",
        purpose: "Find the crash in the settings screen",
        state: "completed",
      },
    ],
  },
  {
    num: 3,
    cwd: "/home/dev/search",
    at: 32,
    agents: agents(8, "Explore", "Benchmark index shard", [
      "completed",
      "working",
      "working",
      "completed",
      "working",
      "failed",
      "working",
      "working",
    ]),
  },
  {
    num: 4,
    cwd: "/home/dev/docs",
    at: 45,
    state: "completed",
    tasks: [3, 3],
    taskText: "Docs page",
  },
  {
    num: 5,
    cwd: "/home/dev/data-pipeline",
    at: 42,
    tasks: [12, 7],
    taskText: "Pipeline stage",
    agents: [
      {
        type: "Plan",
        purpose: "Plan the backfill",
        agents: [
          {
            type: "Explore",
            purpose: "Profile the slow joins",
            state: "completed",
            agents: agents(2, "general-purpose", "Rewrite join", [
              "completed",
              "failed",
            ]),
          },
          {
            type: "Explore",
            purpose: "Check the late events",
            agents: agents(3, "general-purpose", "Replay day", [
              "completed",
              "working",
              "working",
            ]),
          },
        ],
      },
    ],
  },
  {
    num: 6,
    cwd: "/home/dev/infra",
    at: 20,
    agents: [
      {
        type: "general-purpose",
        purpose: "Rotate the staging certificates",
        state: "input_required",
        summary:
          "Claude needs your permission to use Bash: kubectl apply -f certs/",
      },
      {
        type: "Explore",
        purpose: "List the services using old certificates",
        state: "completed",
      },
    ],
  },
  {
    num: 7,
    cwd: "/home/dev/design-system",
    at: 25,
    provider: "codex",
    tasks: [8, 3],
    taskText: "Component token",
  },
  {
    num: 8,
    cwd: "/home/dev/analytics",
    at: 38,
    tasks: [15, 9],
    taskText: "Dashboard",
    agents: [
      {
        type: "Plan",
        purpose: "Plan the dashboards",
        state: "completed",
        agents: agents(4, "general-purpose", "Build dashboard", ["completed"]),
      },
      {
        type: "Plan",
        purpose: "Plan the alerts",
        agents: agents(5, "general-purpose", "Write alert rule", [
          "completed",
          "working",
          "working",
          "completed",
          "working",
        ]),
      },
    ],
  },
  {
    num: 9,
    cwd: "/home/dev/website",
    at: 15,
    agents: agents(2, "Explore", "Audit page speed, part", ["working"]),
  },
  {
    num: 10,
    cwd: "/home/dev/cli",
    at: 10,
    state: "idle",
    tasks: [4, 4],
    taskText: "Flag",
  },
];

// ---------- writing them ----------
// ---------- writing them ----------

const files = sessionFiles();
mkdirSync(out, {recursive: true});
const listed = [];
for (const example of EXAMPLES) {
  let text;
  if (example.generate) text = example.generate();
  else {
    const numbers =
      example.sessions === "all" ? [...files.keys()] : example.sessions;
    const parts = numbers.map(k => {
      const file = files.get(k);
      if (!file) throw new Error(`${example.slug}: no example session ${k}`);
      const body = readFileSync(file, "utf8");
      return body.endsWith("\n") ? body : `${body}\n`;
    });
    // "all" takes the made-up ones too.
    if (example.sessions === "all") parts.push(madeUp(TEAM));
    text = parts.join("");
  }
  writeFileSync(join(out, `${example.slug}.jsonl`), text);
  const {slug, title, description, tags, view, zoomOut} = example;
  // What it opens on: a session of its own, by number, or a made-up one's id.
  // (A made-up one's named by its number: `{madeUp: 0}`.)
  const open =
    example.open && typeof example.open === "object"
      ? madeUpId({num: example.open.madeUp})
      : example.open && fileSessionId(files.get(example.open));
  // The link that opens it: in the view that shows it best, on that session.
  const query = new URLSearchParams({
    ...(view ? {view} : {}),
    ...(zoomOut ? {zoom: "out"} : {}),
  }).toString();
  const href = `/examples/${slug}${query ? `?${query}` : ""}${open ? `#${encodeURIComponent(open)}` : ""}`;
  listed.push({slug, title, description, tags, href});
  console.log(
    `public/examples/${slug}.jsonl: ${text.split("\n").filter(Boolean).length} events`,
  );
}
writeFileSync(
  join(site, "lib/examples.json"),
  `${JSON.stringify(listed, null, 2)}\n`,
);
