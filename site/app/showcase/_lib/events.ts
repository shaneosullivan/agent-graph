import type {AgentEvent} from "./types";

/** An icon, and a few words, for each kind of event. */
export const EVENT_KINDS: Record<string, {icon: string; verb: string}> = {
  "session.started": {icon: "▶", verb: "started a session"},
  "session.ended": {icon: "■", verb: "ended its session"},
  "agent.spawned": {icon: "✚", verb: "started as an agent"},
  "agent.finished": {icon: "✓", verb: "finished"},
  status: {icon: "◉", verb: "changed state"},
  "tasks.updated": {icon: "☑", verb: "updated its task list"},
  "task.upserted": {icon: "☑", verb: "changed a task"},
  "task.deleted": {icon: "✕", verb: "removed a task"},
  "spawn.requested": {icon: "↗", verb: "asked for a helper"},
  "spawn.returned": {icon: "↙", verb: "got its helper's answer"},
  "wait.started": {icon: "⧗", verb: "started waiting"},
  "wait.ended": {icon: "⧖", verb: "stopped waiting"},
  "message.sent": {icon: "✉", verb: "sent a message"},
  activity: {icon: "⚡", verb: "used a tool"},
};

export function eventKind(type: string) {
  return EVENT_KINDS[type] ?? {icon: "•", verb: type};
}

/** A one-line description of what the event says. */
export function eventDetail(e: AgentEvent): string {
  const p = e.payload as Record<string, unknown>;
  const text = (k: string) =>
    typeof p[k] === "string" ? (p[k] as string) : "";
  switch (e.type) {
    case "status":
      return [text("state").replace("_", " "), text("summary")]
        .filter(Boolean)
        .join(": ");
    case "agent.spawned":
    case "spawn.requested":
      return [text("agent_type"), text("purpose")].filter(Boolean).join(": ");
    case "agent.finished":
      return [text("status"), text("summary")].filter(Boolean).join(": ");
    case "tasks.updated":
      return `${Array.isArray(p.items) ? p.items.length : 0} tasks`;
    case "message.sent":
      return text("summary");
    case "wait.started":
      return text("reason");
    case "activity":
      return [text("tool"), text("label")].filter(Boolean).join(": ");
    case "session.started":
      return text("cwd");
    default:
      return "";
  }
}

/** Fields that hold times (Unix milliseconds). */
const TIMES = new Set(["created", "ended", "last_event"]);

/** A value from a change to `field`, short: a time as a time. */
export function short(v: unknown, field = ""): string {
  if (v === null || v === undefined) {
    return "∅";
  }
  if (TIMES.has(field) && typeof v === "number") {
    return new Date(v).toLocaleTimeString(undefined, {
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
    });
  }
  if (typeof v === "string") {
    return v.length > 40 ? `${v.slice(0, 39)}…` : v;
  }
  if (typeof v === "number" || typeof v === "boolean") {
    return String(v);
  }
  const s = JSON.stringify(v);
  return s.length > 40 ? `${s.slice(0, 39)}…` : s;
}

/** Fields that change on almost every event, left out of change lists unless asked for. */
export const NOISY = new Set(["last_event", "headline"]);
