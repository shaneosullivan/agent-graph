/**
 * Browser-side helpers for sharing a log: the same protocol the CLI's
 * `watch-remote` uses (create with the first chunk, then append the rest).
 */

/** Chunks sent per request; the API accepts up to 512 KB. */
const CHUNK_BYTES = 256 * 1024;

export type Summary = { events: number; sessions: number; skipped: number };

/**
 * Counts the events (and the sessions they belong to) in some JSON Lines. A
 * keyframe (in a log from the site) stands for events, and isn't one.
 */
export function summarize(text: string): Summary {
  const sessions = new Set<string>();
  let events = 0;
  let skipped = 0;
  for (const line of text.split("\n")) {
    if (!line.trim()) continue;
    try {
      const e = JSON.parse(line);
      if (e && e.type === "keyframe") continue;
      // What the site reads as an event (src/event.rs's Envelope).
      const event =
        e &&
        Number.isInteger(e.v) &&
        e.v >= 0 &&
        [e.id, e.ts, e.type, e.node].every((field) => typeof field === "string");
      if (event) {
        events++;
        sessions.add(e.node.split("/")[0]);
      } else {
        skipped++;
      }
    } catch {
      skipped++;
    }
  }
  return { events, sessions: sessions.size, skipped };
}

/** Splits text into chunks of at most CHUNK_BYTES, only at line boundaries. */
export function chunks(text: string): string[] {
  const enc = new TextEncoder();
  const out: string[] = [];
  let current = "";
  let size = 0;
  for (const raw of text.split("\n")) {
    if (!raw.trim()) continue;
    const line = raw + "\n";
    const bytes = enc.encode(line).length;
    if (size + bytes > CHUNK_BYTES && current) {
      out.push(current);
      current = "";
      size = 0;
    }
    current += line;
    size += bytes;
  }
  if (current) out.push(current);
  return out;
}

function base64url(text: string): string {
  let binary = "";
  for (const byte of new TextEncoder().encode(text)) binary += String.fromCharCode(byte);
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

async function check(res: Response): Promise<Response> {
  if (!res.ok) throw new Error((await res.text()) || `The site returned ${res.status}`);
  return res;
}

/**
 * Uploads `text` as a new log (cut first: lib/trim.ts), and returns its id.
 * `onProgress` gets the number of chunks sent so far and the total.
 */
export async function share(
  text: string,
  opts: { source: "paste" | "upload"; password?: string; onProgress?: (sent: number, total: number) => void },
): Promise<string> {
  const parts = chunks(text);
  if (!parts.length) throw new Error("There's nothing to share.");
  const enc = new TextEncoder();

  const headers: Record<string, string> = {
    "Content-Type": "application/x-ndjson",
    "X-Agent-Graph-Source": opts.source,
  };
  if (opts.password) headers["X-Agent-Graph-Password"] = base64url(opts.password);
  const created = (await (
    await check(await fetch("/api/logs", { method: "POST", headers, body: parts[0] }))
  ).json()) as { id: string; writeToken: string };
  opts.onProgress?.(1, parts.length);

  let offset = enc.encode(parts[0]).length;
  for (let i = 1; i < parts.length; i++) {
    await check(
      await fetch(`/api/logs/${created.id}/append?offset=${offset}`, {
        method: "POST",
        headers: { "Content-Type": "application/x-ndjson", Authorization: `Bearer ${created.writeToken}` },
        body: parts[i],
      }),
    );
    offset += enc.encode(parts[i]).length;
    opts.onProgress?.(i + 1, parts.length);
  }
  return created.id;
}
