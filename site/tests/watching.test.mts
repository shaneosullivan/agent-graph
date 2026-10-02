// Unit tests for lib/watching.ts:  npm run test:unit

import assert from "node:assert/strict";
import {test} from "node:test";

const {watchingOf, watchingText} = await import("../lib/watching.ts");

const share = (
  host: string,
  summary: Record<string, unknown>,
  sessions = Object.keys(summary).length,
) => ({id: `log-${host}`, host, at: Date.now(), sessions, summary});

const s = (state: string, provider = "claude-code") => ({state, provider});

test("nothing running isn't watching", () => {
  assert.deepEqual(watchingOf([]), {watching: false});
});

test("active and completed sessions, counted apart", () => {
  const w = watchingOf([
    share("mac", {
      a: s("working"),
      b: s("input_required"),
      c: s("idle"),
      d: s("completed"),
      e: s("failed"),
      f: s("canceled"),
    }),
  ]);
  assert.deepEqual(w, {
    watching: true,
    sessions: 6,
    active: 3,
    completed: 3,
    agents: ["Claude Code"],
    computers: 1,
    clouds: [],
  });
  assert.deepEqual(watchingText(w as never), {
    title: "Watching 3 active sessions, live",
    lines: [
      "3 completed in the last day.",
      "agent-graph watch-remote is running on your computer.",
    ],
  });
});

test("only the summaries' sessions: not the ones started under them", () => {
  // 24 session files, of which 2 are top-level sessions.
  const w = watchingOf([
    share("mac", {a: s("working"), b: s("completed")}, 24),
  ]);
  assert.equal(w.watching && w.sessions, 2);
});

test("a share from before summaries counts its sessions as active", () => {
  const w = watchingOf([share("mac", {}, 4)]);
  assert.equal(w.watching && w.active, 4);
});

test("more than one coding agent, and a cloud, are said", () => {
  const w = watchingOf([
    share("mac", {a: s("working", "codex"), b: s("working")}),
    share("Codex cloud", {c: s("completed", "codex"), d: s("idle", "run")}),
  ]);
  assert.deepEqual(watchingText(w as never), {
    title: "Watching 3 active sessions, live",
    lines: [
      "1 completed in the last day. From Claude Code & Codex.",
      "agent-graph watch-remote is running on your computer and in Codex cloud.",
    ],
  });
});

test("only in the cloud, on two computers, and one active session", () => {
  const cloud = watchingOf([share("Claude Code cloud", {a: s("working")})]);
  assert.deepEqual(watchingText(cloud as never), {
    title: "Watching 1 active session, live",
    lines: ["agent-graph watch-remote is running in Claude Code cloud."],
  });
  const two = watchingOf([
    share("mac", {a: s("completed")}),
    share("linux", {b: s("completed")}),
  ]);
  assert.deepEqual(watchingText(two as never), {
    title: "Watching, live: no session is active",
    lines: [
      "2 completed in the last day.",
      "agent-graph watch-remote is running on 2 computers.",
    ],
  });
});

test("no session in the last day", () => {
  const w = watchingOf([share("mac", {}, 0)]);
  assert.deepEqual(watchingText(w as never), {
    title: "Watching, live",
    lines: [
      "agent-graph watch-remote is running on your computer; no session has had an event in the last day.",
    ],
  });
});
