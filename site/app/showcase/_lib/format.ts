import type {AgentNode, NodeState} from "./types";

/** Each state's colour and name, the same everywhere in the app. */
export const STATES: Record<
  NodeState,
  {color: string; label: string; hint: string}
> = {
  working: {
    color: "#5b8cff",
    label: "Working",
    hint: "Doing something right now.",
  },
  input_required: {
    color: "#ffb547",
    label: "Needs you",
    hint: "Waiting on a person: a question, a permission, a plan.",
  },
  idle: {
    color: "#8b90a8",
    label: "Idle",
    hint: "A session between turns, waiting for its next prompt.",
  },
  completed: {color: "#34c98a", label: "Completed", hint: "Finished."},
  failed: {color: "#ff5f6d", label: "Failed", hint: "Finished with an error."},
  canceled: {
    color: "#5d6275",
    label: "Canceled",
    hint: "Stopped before it finished.",
  },
};

export const STATE_ORDER: Array<NodeState> = [
  "working",
  "input_required",
  "idle",
  "completed",
  "failed",
  "canceled",
];

export const STALE_COLOR = "#ff7a45";
export const BLOCKED_COLOR = "#c084fc";

/** A node's name: its title, its agent type, the folder a session runs in, or its provider's id for it. */
export function nodeName(
  n: Pick<AgentNode, "title" | "agent_type" | "kind" | "provider_ref"> & {
    cwd?: string | null;
  },
): string {
  if (n.title) {
    return n.title;
  }
  if (n.agent_type) {
    return n.agent_type;
  }
  const folder = n.cwd?.split(/[/\\]/).filter(Boolean).pop();
  if (n.kind === "session" && folder) {
    return folder;
  }
  const tail = n.provider_ref.split(/[:/]/).pop() ?? n.provider_ref;
  return `${n.kind === "session" ? "Session" : "Agent"} ${tail.slice(0, 8)}`;
}

/** What it's doing, in a line. */
export function nodeLine(
  n: Pick<
    AgentNode,
    "headline" | "summary" | "purpose" | "attention" | "state"
  >,
): string {
  if (n.state === "input_required" && n.attention) {
    return n.attention;
  }
  return n.headline ?? n.summary ?? n.purpose ?? "";
}

let reference: number | null = null;

/**
 * "Now", for saying how long ago something was: the clock; or, for the
 * public showcase's demo graphs, which are read as they stood at their last
 * events (lib/client.tsx), the latest of those events' times: the demo's
 * graphs share a clock.
 */
export function now(): number {
  return reference ?? Date.now();
}

export function setNow(ms: number | null): void {
  reference = ms === null ? null : Math.max(reference ?? 0, ms);
}

export function ago(ms: number, at = now()): string {
  const s = Math.max(0, Math.round((at - ms) / 1000));
  if (s < 60) {
    return `${s}s ago`;
  }
  const m = Math.round(s / 60);
  if (m < 60) {
    return `${m} min ago`;
  }
  const h = Math.round(m / 60);
  if (h < 48) {
    return `${h} h ago`;
  }
  return `${Math.round(h / 24)} days ago`;
}

export function duration(ms: number): string {
  if (ms < 1000) {
    return `${Math.round(ms)} ms`;
  }
  const s = ms / 1000;
  if (s < 60) {
    return `${s.toFixed(s < 10 ? 1 : 0)} s`;
  }
  const m = s / 60;
  if (m < 60) {
    return `${m.toFixed(m < 10 ? 1 : 0)} min`;
  }
  const h = m / 60;
  return `${h.toFixed(h < 10 ? 1 : 0)} h`;
}

export function clock(ms: number): string {
  return new Date(ms).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

export function shortTime(ms: number): string {
  return new Date(ms).toLocaleTimeString(undefined, {
    hour: "2-digit",
    minute: "2-digit",
  });
}

/** A time for an axis: with its day too, when the axis spans more than one. */
export function axisTime(ms: number, span: number): string {
  if (span < 20 * 60 * 60 * 1000) {
    return shortTime(ms);
  }
  return new Date(ms).toLocaleString(undefined, {
    day: "numeric",
    month: "short",
    hour: "2-digit",
    minute: "2-digit",
  });
}

/** How long it ran (so far, if it's still going: to `now`). */
export function ranFor(
  n: Pick<AgentNode, "created" | "ended" | "last_event">,
  now?: number,
): number {
  return (n.ended ?? now ?? n.last_event) - n.created;
}

export function plural(n: number, one: string, many = `${one}s`): string {
  return `${n.toLocaleString()} ${n === 1 ? one : many}`;
}

/** The median of `values` (0 for none). */
export function median(values: Array<number>): number {
  if (!values.length) {
    return 0;
  }
  const s = [...values].sort((a, b) => a - b);
  const mid = Math.floor(s.length / 2);
  return s.length % 2 ? s[mid] : (s[mid - 1] + s[mid]) / 2;
}
