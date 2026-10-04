import type {Share} from "./store";

/**
 * What GET /api/watching says of an account's running `watch-remote`s, for
 * the home page's card (app/watch-card.tsx): how many sessions they're
 * watching, how many of those are still going and how many are over, which
 * coding agents they're from, and where they're running.
 */
export type Watching =
  | {
      watching: true;
      /** The sessions they're watching: `active` + `completed`. */
      sessions: number;
      /** Still going: working, waiting on you, or between turns. */
      active: number;
      /** Over: completed, failed or canceled. */
      completed: number;
      /** The coding agents they're from, as named ("Claude Code", "Codex"). */
      agents: Array<string>;
      /** How many computers they're running on (not in a cloud). */
      computers: number;
      /** The clouds they're running in ("Claude Code cloud", "Codex cloud", "Cursor cloud"). */
      clouds: Array<string>;
    }
  | {watching: false};

/** A session's states once it's over (the CLI's `State::is_terminal`). */
const OVER = new Set(["completed", "failed", "canceled"]);

/** The coding agents by provider; others (`run`'s commands) aren't named. */
const AGENTS: Record<string, string> = {
  "claude-code": "Claude Code",
  codex: "Codex",
  cursor: "Cursor",
};

/**
 * The names `watch-remote` gives as its host in a coding agent's cloud
 * (the CLI's `account::host_name`), rather than the computer's.
 */
const CLOUDS = ["Claude Code cloud", "Codex cloud", "Cursor cloud"];

/**
 * What `running` (the account's shares whose `watch-remote` is running
 * now) are watching. Each share's summary has its sessions, as /watch
 * lists them (the most recent 100, without the sessions started under
 * them), each with its state and provider. A share from before summaries
 * (0.1.5) only has a count, of every session's file, which is taken as
 * active.
 */
export function watchingOf(running: Array<Share>): Watching {
  if (running.length === 0) {
    return {watching: false};
  }
  let active = 0;
  let completed = 0;
  const agents = new Set<string>();
  const clouds = new Set<string>();
  const computers = new Set<string>();
  for (const share of running) {
    const summaries = Object.values(share.summary);
    for (const s of summaries) {
      const {state, provider} = (s ?? {}) as {
        state?: unknown;
        provider?: unknown;
      };
      if (typeof state === "string" && OVER.has(state)) {
        completed++;
      } else {
        active++;
      }
      if (typeof provider === "string" && AGENTS[provider]) {
        agents.add(AGENTS[provider]);
      }
    }
    if (summaries.length === 0) {
      active += share.sessions;
    }
    if (CLOUDS.includes(share.host)) {
      clouds.add(share.host);
    } else {
      computers.add(share.host || share.id);
    }
  }
  return {
    watching: true,
    sessions: active + completed,
    active,
    completed,
    // In the order AGENTS has them, so it reads the same each time.
    agents: Object.values(AGENTS).filter(a => agents.has(a)),
    computers: computers.size,
    clouds: CLOUDS.filter(c => clouds.has(c)),
  };
}

const count = (n: number, one: string, many = `${one}s`) =>
  `${n} ${n === 1 ? one : many}`;

/** "a", "a and b", "a, b and c". */
const and = (words: Array<string>) =>
  words.length < 2
    ? words.join("")
    : `${words.slice(0, -1).join(", ")} and ${words[words.length - 1]}`;

/**
 * What the home page's card says of `w` (a running `watch-remote`): a
 * title, with how many sessions are active, and the lines under it: how
 * many have completed, which coding agents they're from when there's more
 * than one, and where it's running.
 *
 *   Watching 3 active sessions, live
 *   21 completed in the last day. From Claude Code & Codex.
 *   agent-graph watch-remote is running on your computer and in Codex cloud.
 */
export function watchingText(w: Extract<Watching, {watching: true}>): {
  title: string;
  lines: Array<string>;
} {
  const where = and([
    ...(w.computers === 1
      ? ["on your computer"]
      : w.computers > 1
        ? [`on ${w.computers} computers`]
        : []),
    ...w.clouds.map(c => `in ${c}`),
  ]);
  const running = `agent-graph watch-remote is running ${where || "on your computer"}.`;
  if (w.sessions === 0) {
    return {
      title: "Watching, live",
      lines: [
        `${running.slice(0, -1)}; no session has had an event in the last day.`,
      ],
    };
  }
  const about = [
    ...(w.completed > 0 ? [`${w.completed} completed in the last day.`] : []),
    ...(w.agents.length > 1 ? [`From ${w.agents.join(" & ")}.`] : []),
  ].join(" ");
  return {
    title:
      w.active > 0
        ? `Watching ${count(w.active, "active session")}, live`
        : "Watching, live: no session is active",
    lines: about ? [about, running] : [running],
  };
}
