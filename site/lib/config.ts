/** Limits and settings shared by the API routes. */

/**
 * The largest body one request may carry. The CLI sends at most 256 KB, cut
 * at line boundaries; Firestore documents top out at 1 MiB.
 */
export const MAX_CHUNK_BYTES = 512 * 1024;

/** A log not in use is deleted (lib/cleanup.ts); its sharer stops, and says so. */
export function gone(): Response {
  return new Response("This log has expired: it had no new events for a week, so it was deleted.", { status: 410 });
}

/**
 * A request's body, as text, exactly as it was sent: a byte-order mark at
 * its start is kept (`req.text()` drops one), so what's stored is the bytes
 * sent, and chunks' offsets stay true.
 */
export async function bodyText(req: Request): Promise<string> {
  return new TextDecoder("utf-8", { ignoreBOM: true }).decode(await req.arrayBuffer());
}

/**
 * The most text a log may store: counted as its chunks are stored and
 * trimmed (lib/store.ts), so a live share that trims its start can go on
 * for good, but chunks that overlap each count in full. Appends are also
 * kept within this of where the log starts, judged from their offsets.
 */
export const MAX_LOG_BYTES = 64 * 1024 * 1024;

/** Chunks returned per content request; the viewer asks again for more. */
export const CHUNKS_PER_READ = 200;

/**
 * Bytes per content request, give or take one chunk: a read stops once it
 * has this many, and the viewer asks again for more. (Vercel refuses
 * responses over 4.5 MB, and each read passes through the function's
 * memory.)
 */
export const BYTES_PER_READ = 2 * 1024 * 1024;

/**
 * Chunks fetched per database query while reading. A query's chunks are all
 * held at once, so this bounds what a read holds (16 × MAX_CHUNK_BYTES =
 * 8 MiB, besides the up to BYTES_PER_READ + MAX_CHUNK_BYTES it returns)
 * however big the log's chunks are.
 */
export const CHUNKS_PER_QUERY = 16;

/**
 * Wrong password guesses allowed in each window (every guess costs a scrypt
 * run): per log from one address, per log from anywhere, and per address
 * across logs. Past any of them, guesses wait for the window to end, even
 * with the right password. A right one doesn't count.
 */
export const UNLOCKS_PER_LOG_AND_ADDRESS = 5;
export const UNLOCKS_PER_LOG = 20;
export const UNLOCKS_PER_ADDRESS = 30;
export const UNLOCK_WINDOW_MS = 15 * 60 * 1000;
/** How long to wait when too many guesses arrive at once to count them. */
export const UNLOCK_BUSY_SECONDS = 5;

/** Log ids: 12 base62 characters (about 71 bits). */
export const ID_PATTERN = /^[A-Za-z0-9]{12}$/;

export type Source = "watch" | "paste" | "upload";
export const SOURCES: readonly Source[] = ["watch", "paste", "upload"];

/** The public origin for links, e.g. https://agentgraph.chofter.com. */
export function siteUrl(req: Request): string {
  const configured = process.env.NEXT_PUBLIC_SITE_URL?.replace(/\/+$/, "");
  return configured || new URL(req.url).origin;
}

/** The cookie that lets a browser read a password-protected log. */
export function viewCookieName(id: string): string {
  return `ag_v_${id}`;
}

/**
 * Where a request came from, as the platform reports it, for counting
 * password guesses (see `addressBlock`); null if it doesn't say. (Vercel sets
 * X-Real-IP itself; elsewhere a client could, so the per-address limits are
 * only as good as the proxy in front of the site. The per-log one holds
 * regardless.)
 */
export function clientAddress(req: Request): string | null {
  const real = req.headers.get("x-real-ip")?.trim();
  const forwarded = req.headers.get("x-forwarded-for")?.split(",")[0]?.trim();
  const address = real || forwarded;
  return address ? addressBlock(address) : null;
}

/**
 * The block an address is counted by: an IPv4 address itself, an IPv6 one
 * by its /64 (one household, or one server, has a whole /64 to move around
 * in).
 */
export function addressBlock(address: string): string {
  const withPort = /^(\d+\.\d+\.\d+\.\d+):\d+$/.exec(address);
  if (withPort) return withPort[1];
  const bare = address.replace(/^\[|\](:\d+)?$/g, "").replace(/%.*$/, "");
  if (!bare.includes(":")) return bare;
  // Its groups, as numbers; an IPv4 address at the end (::ffff:1.2.3.4) is two.
  const parse = (part: string) =>
    (part ? part.split(":") : []).flatMap((g) => {
      if (!g.includes(".")) return [Number.parseInt(g, 16) || 0];
      const [a, b, c, d] = g.split(".").map((n) => Number(n) || 0);
      return [(a << 8) | b, (c << 8) | d];
    });
  const [head, tail] = bare.split("::");
  const front = parse(head);
  const back = tail === undefined ? [] : parse(tail);
  const groups =
    tail === undefined ? front : [...front, ...Array(Math.max(0, 8 - front.length - back.length)).fill(0), ...back];
  // IPv4 mapped into IPv6, however it's written: the IPv4 address.
  if (groups.length === 8 && groups.slice(0, 5).every((g) => g === 0) && groups[5] === 0xffff) {
    return [groups[6] >> 8, groups[6] & 255, groups[7] >> 8, groups[7] & 255].join(".");
  }
  return `${groups
    .slice(0, 4)
    .map((g) => g.toString(16))
    .join(":")}::/64`;
}

/** Whether the request came over HTTPS (directly or via a proxy). */
export function isHttps(req: Request): boolean {
  return req.headers.get("x-forwarded-proto") === "https" || new URL(req.url).protocol === "https:";
}
