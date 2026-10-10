// The API's ids (site/openapi.json, "Ids"): each object's own id, with a
// prefix saying what it is. Each is made from the id it stands for, and read
// back to it, with no lookup.

const LOG_ID = /^[A-Za-z0-9]{12}$/;

/** A graph's id: `gph_` and its log's id (the id in its link). */
export function graphId(log: string): string {
  return `gph_${log}`;
}

/** The log a graph id (or a share link's id, which works too) names; null if it isn't one. */
export function parseGraphId(id: string): string | null {
  const log = id.startsWith("gph_") ? id.slice(4) : id;
  return LOG_ID.test(log) ? log : null;
}

/**
 * A node's id: `node_` and the base64url (no padding) of the id its agent
 * tool gave it (`claude-code:<session>/<agent>`), which has `/` and `:` in
 * it, so it's safe in a URL's path.
 */
export function nodeId(ref: string): string {
  return `node_${Buffer.from(ref, "utf8").toString("base64url")}`;
}

/** The provider's id a node id stands for; null if it isn't one (or isn't written as `nodeId` writes it). */
export function parseNodeId(id: string): string | null {
  if (!/^node_[A-Za-z0-9_-]+$/.test(id)) {
    return null;
  }
  const ref = Buffer.from(id.slice(5), "base64url").toString("utf8");
  // Exactly as written, so each node has one id.
  return ref && nodeId(ref) === id ? ref : null;
}

/** An event's id: `evt_` and the event's own (a ULID, which sorts by time). */
export function eventId(id: string): string {
  return `evt_${id}`;
}

/** The event id an API event id stands for; null if it isn't one. */
export function parseEventId(id: string): string | null {
  return /^evt_[A-Za-z0-9]{1,64}$/.test(id) ? id.slice(4) : null;
}
