/** Limits and settings shared by the API routes. */

/**
 * The largest body one request may carry. The CLI sends at most 256 KB, cut
 * at line boundaries; Firestore documents top out at 1 MiB.
 */
export const MAX_CHUNK_BYTES = 512 * 1024;

/** The largest a log may grow, judged from the offset of each append. */
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

/** Whether the request came over HTTPS (directly or via a proxy). */
export function isHttps(req: Request): boolean {
  return req.headers.get("x-forwarded-proto") === "https" || new URL(req.url).protocol === "https:";
}
