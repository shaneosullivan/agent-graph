import { FieldPath, Timestamp } from "firebase-admin/firestore";

import { CHUNKS_PER_READ, type Source } from "./config";
import { decryptChunk, encryptChunk } from "./encryption";
import { firestore } from "./firebase";

/**
 * How logs are kept in Firestore:
 *
 *   logs/{id}                 { source, pw?, createdAt }
 *   logs/{id}/chunks/{offset} { e: <encrypted JSON Lines, bytes> }
 *
 * Chunk text is encrypted before it's stored (lib/encryption.ts); Firestore
 * never sees it in the clear. The metadata holds nothing sensitive: the
 * password is only a scrypt hash.
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

/** A chunk's document id: its offset, zero-padded to 15 digits. */
export function chunkKey(offset: number): string {
  return String(offset).padStart(15, "0");
}

export class IdTaken extends Error {}

/** Creates log `id` with its first chunk, all in one commit. */
export async function createLog(id: string, meta: Omit<Meta, "createdAt">, text: string): Promise<void> {
  const log = logs().doc(id);
  const batch = firestore().batch();
  // `create` fails if the id is already taken, rather than overwriting it.
  // (Firestore rejects undefined fields, so `pw` is only set when present.)
  batch.create(log, { source: meta.source, ...(meta.pw ? { pw: meta.pw } : {}), createdAt: Timestamp.now() });
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
  const doc = logs().doc(id).collection("chunks").doc(key);
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
    // The log's id is what lets people read it, so it isn't logged.
    console.error(`A stored chunk (${key}) couldn't be decrypted; refused the append there.`);
    return false;
  }
}

// Metadata never changes after creation, so each server instance keeps what
// it has read. That saves a read on every viewer poll.
const metaCache = new Map<string, Meta | null>();
const META_CACHE_LIMIT = 5000;

export async function getMeta(id: string): Promise<Meta | null> {
  const cached = metaCache.get(id);
  if (cached) return cached;
  const snap = await logs().doc(id).get();
  const meta = snap.exists ? (snap.data() as Meta) : null;
  // Don't cache "not found": the log may be created a moment later.
  if (meta) {
    if (metaCache.size >= META_CACHE_LIMIT) metaCache.delete(metaCache.keys().next().value!);
    metaCache.set(id, meta);
  }
  return meta;
}

/**
 * The chunks after `after` (a chunk key, or "" for the start), joined in log
 * order. `last` is the key to pass as `after` next time; `more` says whether
 * there are more chunks right now.
 */
export async function readChunks(
  id: string,
  after: string,
): Promise<{ text: string; last: string | null; more: boolean }> {
  let query = logs().doc(id).collection("chunks").orderBy(FieldPath.documentId()).limit(CHUNKS_PER_READ);
  if (after) query = query.startAfter(after);
  const snap = await query.get();
  const text = snap.docs.map((doc) => decryptChunk(id, doc.id, doc.get("e"))).join("");
  const last = snap.docs.length ? snap.docs[snap.docs.length - 1].id : null;
  return { text, last, more: snap.docs.length === CHUNKS_PER_READ };
}
