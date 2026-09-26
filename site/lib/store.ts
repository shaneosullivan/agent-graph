import { FieldPath, Timestamp } from "firebase-admin/firestore";

import { BYTES_PER_READ, CHUNKS_PER_QUERY, CHUNKS_PER_READ, type Source } from "./config";
import { safeEqual } from "./crypto";
import { decryptChunk, encryptChunk, metaTag, storageId } from "./encryption";
import { firestore } from "./firebase";

/**
 * How logs are kept in Firestore:
 *
 *   logs/{sid}                 { source, pw?, createdAt, mac }
 *   logs/{sid}/chunks/{offset} { e: <encrypted JSON Lines, bytes> }
 *
 * `sid` is an HMAC of the log's id (lib/encryption.ts), not the id itself:
 * the id is the link that opens the log, so the database doesn't hold it.
 * Chunk text is encrypted before it's stored; Firestore never sees it in the
 * clear. The metadata holds nothing sensitive (the password is only a scrypt
 * hash), and `mac` binds it to the log's id, so it can't be changed or
 * copied from another log without being refused.
 *
 * Each chunk is a separate document whose id is its byte offset in the log,
 * zero-padded so ids sort in log order. Appending is therefore one write of a
 * new document: nothing is read, and nothing already stored is rewritten, so
 * the cost doesn't grow with the log. Readers page through chunks in id
 * order, and never read one twice, so a chunk never changes: a retry of the
 * same bytes is accepted, and different bytes at an offset already stored
 * are refused (checking costs a read, only then).
 */

export type Meta = {
  source: Source;
  /** scrypt hash of the viewer password, if there is one. */
  pw?: string;
  createdAt: Timestamp;
};

const logs = () => firestore().collection("logs");

/** Log `id`'s document. */
const logDoc = (id: string) => logs().doc(storageId(id));

/** A chunk's document id: its offset, zero-padded to 15 digits. */
export function chunkKey(offset: number): string {
  return String(offset).padStart(15, "0");
}

export class IdTaken extends Error {}

/** Creates log `id` with its first chunk, all in one commit. */
export async function createLog(id: string, meta: Omit<Meta, "createdAt">, text: string): Promise<void> {
  const log = logDoc(id);
  const batch = firestore().batch();
  // `create` fails if the id is already taken, rather than overwriting it.
  // (Firestore rejects undefined fields, so `pw` is only set when present.)
  batch.create(log, {
    source: meta.source,
    ...(meta.pw ? { pw: meta.pw } : {}),
    createdAt: Timestamp.now(),
    mac: metaTag(id, meta),
  });
  if (text) {
    const key = chunkKey(0);
    batch.set(log.collection("chunks").doc(key), { e: encryptChunk(id, key, text) });
  }
  try {
    await batch.commit();
  } catch (err) {
    // gRPC ALREADY_EXISTS
    if ((err as { code?: number }).code === 6) throw new IdTaken(id);
    throw err;
  }
}

export class ChunkTaken extends Error {}

/**
 * Encrypts and stores one chunk at `offset`. One write, no reads, unless a
 * chunk is already stored there: then it's read, and anything but the same
 * bytes (or one that can't be decrypted) is refused with `ChunkTaken`.
 */
export async function appendChunk(id: string, offset: number, text: string): Promise<void> {
  const key = chunkKey(offset);
  const doc = logDoc(id).collection("chunks").doc(key);
  try {
    await doc.create({ e: encryptChunk(id, key, text) });
  } catch (err) {
    // gRPC ALREADY_EXISTS
    if ((err as { code?: number }).code !== 6) throw err;
    const stored = (await doc.get()).get("e");
    if (!stored || !holds(id, key, stored, text)) throw new ChunkTaken(key);
  }
}

/**
 * Whether a stored chunk holds `text`. One that can't be decrypted doesn't:
 * refusing it tells the client to stop, where an error would have it retry
 * for good.
 */
function holds(id: string, key: string, stored: Uint8Array, text: string): boolean {
  try {
    return decryptChunk(id, key, stored) === text;
  } catch {
    // Named by where it's stored: the log's id is what lets people read it.
    console.error(`Chunk logs/${storageId(id)}/chunks/${key} couldn't be decrypted; refused the append there.`);
    return false;
  }
}

// Metadata never changes after creation, so each server instance keeps what
// it has read. That saves a read on every viewer poll.
const metaCache = new Map<string, Meta | null>();
const META_CACHE_LIMIT = 5000;

/** Log `id`'s metadata; null if there's no such log, or if it was changed. */
export async function getMeta(id: string): Promise<Meta | null> {
  const cached = metaCache.get(id);
  if (cached) return cached;
  const snap = await logDoc(id).get();
  let meta: Meta | null = null;
  if (snap.exists) {
    const { source, pw, createdAt, mac } = snap.data() as Meta & { mac?: unknown };
    if (typeof mac === "string" && safeEqual(mac, metaTag(id, { source, pw }))) {
      meta = { source, ...(pw ? { pw } : {}), createdAt };
    } else {
      // Named by where it's stored: the log's id is what lets people read it.
      console.error(`The metadata of logs/${storageId(id)} doesn't match its tag; treated as missing.`);
    }
  }
  // Don't cache "not found": the log may be created a moment later.
  if (meta) {
    if (metaCache.size >= META_CACHE_LIMIT) metaCache.delete(metaCache.keys().next().value!);
    metaCache.set(id, meta);
  }
  return meta;
}

/**
 * The chunks after `after` (a chunk key, or "" for the start), joined in log
 * order, up to `CHUNKS_PER_READ` of them or `BYTES_PER_READ` (and the chunk
 * that crosses it). `last` is the key to pass as `after` next time; `more`
 * says whether there may be more chunks right now.
 *
 * They're fetched `CHUNKS_PER_QUERY` at a time, so however the log's chunks
 * were written, a read holds at most that many at once, and stops at the
 * byte budget having fetched at most that many more than it used.
 */
export async function readChunks(
  id: string,
  after: string,
): Promise<{ text: string; last: string | null; more: boolean }> {
  const chunks = logDoc(id).collection("chunks").orderBy(FieldPath.documentId());
  const texts: string[] = [];
  let bytes = 0;
  let last: string | null = null;
  while (texts.length < CHUNKS_PER_READ) {
    const want = Math.min(CHUNKS_PER_QUERY, CHUNKS_PER_READ - texts.length);
    const cursor: string = last ?? after;
    const snap = await (cursor ? chunks.startAfter(cursor) : chunks).limit(want).get();
    for (const doc of snap.docs) {
      const text = decryptChunk(id, doc.id, doc.get("e"));
      texts.push(text);
      last = doc.id;
      bytes += Buffer.byteLength(text);
      if (bytes >= BYTES_PER_READ) return { text: texts.join(""), last, more: true };
    }
    if (snap.size < want) return { text: texts.join(""), last, more: false };
  }
  return { text: texts.join(""), last, more: true };
}
