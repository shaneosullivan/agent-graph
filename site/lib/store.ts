import { FieldPath, Timestamp } from "firebase-admin/firestore";

import {
  BYTES_PER_READ,
  CHUNKS_PER_QUERY,
  CHUNKS_PER_READ,
  type Source,
  UNLOCK_BUSY_SECONDS,
  UNLOCK_WINDOW_MS,
  UNLOCKS_PER_ADDRESS,
  UNLOCKS_PER_LOG,
  UNLOCKS_PER_LOG_AND_ADDRESS,
} from "./config";
import { addressKey, safeEqual } from "./crypto";
import { decryptChunk, encryptChunk, metaTag, storageId } from "./encryption";
import { firestore } from "./firebase";

/**
 * How logs are kept in Firestore:
 *
 *   logs/{sid}                 { source, pw?, createdAt, mac }
 *   logs/{sid}/chunks/{offset} { e: <encrypted JSON Lines, bytes> }
 *   unlock-attempts/{bucket}   { n, since, expireAt }  (password guesses)
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
 *
 * A live share keeps only its last two keyframes' worth (see `trimLog`), so a
 * log's first chunk isn't always at offset 0.
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

/** Chunks deleted in one batch, which Firestore applies all at once. */
const TRIM_BATCH = 500;

/**
 * Deletes log `id`'s chunks that start before offset `before`: a live share
 * keeps only its last two keyframes' worth, and when it sends a keyframe,
 * asks for everything before the previous one to go. They're deleted
 * newest first, a batch at a time (each all at once), so a reader never
 * starts partway through what's being deleted; one that's reading across it
 * sees a gap, and starts again (site-source.js). Returns how many went.
 */
export async function trimLog(id: string, before: number): Promise<number> {
  // Their keys, oldest first (Firestore can't scan keys the other way).
  const chunks = logDoc(id).collection("chunks");
  const older = chunks.orderBy(FieldPath.documentId()).endBefore(chunkKey(before)).select();
  const keys: string[] = [];
  for (;;) {
    const page = keys.length ? older.startAfter(keys[keys.length - 1]) : older;
    const snap = await page.limit(TRIM_BATCH).get();
    keys.push(...snap.docs.map((doc) => doc.id));
    if (snap.size < TRIM_BATCH) break;
  }
  for (let end = keys.length; end > 0; end -= TRIM_BATCH) {
    const batch = firestore().batch();
    for (const key of keys.slice(Math.max(0, end - TRIM_BATCH), end)) batch.delete(chunks.doc(key));
    await batch.commit();
  }
  return keys.length;
}

/** Where log `id`'s first chunk (the oldest kept) starts, or null if it has none. */
export async function firstChunkOffset(id: string): Promise<number | null> {
  const snap = await logDoc(id).collection("chunks").orderBy(FieldPath.documentId()).limit(1).select().get();
  return snap.empty ? null : Number(snap.docs[0].id);
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
 * that crosses it), stopping at a gap: chunks are stored one after another,
 * so a gap means the log's start was trimmed while it was read (see
 * `trimLog`). `first` is the first one's key (where the text starts);
 * `last` is the key to pass as `after` next time; `more` says whether there
 * may be more chunks right now.
 *
 * They're fetched `CHUNKS_PER_QUERY` at a time, so however the log's chunks
 * were written, a read holds at most that many at once, and stops at the
 * byte budget having fetched at most that many more than it used.
 */
export async function readChunks(
  id: string,
  after: string,
): Promise<{ text: string; first: string | null; last: string | null; more: boolean }> {
  const chunks = logDoc(id).collection("chunks").orderBy(FieldPath.documentId());
  const texts: string[] = [];
  let bytes = 0;
  let first: string | null = null;
  let last: string | null = null;
  // Where the next chunk should start.
  let next: number | null = null;
  while (texts.length < CHUNKS_PER_READ) {
    const want = Math.min(CHUNKS_PER_QUERY, CHUNKS_PER_READ - texts.length);
    const cursor: string = last ?? after;
    const snap = await (cursor ? chunks.startAfter(cursor) : chunks).limit(want).get();
    for (const doc of snap.docs) {
      if (next !== null && Number(doc.id) !== next) return { text: texts.join(""), first, last, more: true };
      const text = decryptChunk(id, doc.id, doc.get("e"));
      texts.push(text);
      first ??= doc.id;
      last = doc.id;
      next = Number(doc.id) + Buffer.byteLength(text);
      bytes += Buffer.byteLength(text);
      if (bytes >= BYTES_PER_READ) return { text: texts.join(""), first, last, more: true };
    }
    if (snap.size < want) return { text: texts.join(""), first, last, more: false };
  }
  return { text: texts.join(""), first, last, more: true };
}

/** A guess counted by `takeUnlockAttempt`, to give back if it was right. */
export type Reservation = { ref: FirebaseFirestore.DocumentReference; since: number }[];

/** The buckets a guess at log `id` from `address` counts against. */
function unlockBuckets(id: string, address: string | null) {
  const attempts = firestore().collection("unlock-attempts");
  const sid = storageId(id);
  // All keyed: the documents name neither the log nor the address.
  const buckets = [{ ref: attempts.doc(`log-${sid}`), limit: UNLOCKS_PER_LOG }];
  if (address) {
    buckets.push(
      { ref: attempts.doc(`pair-${addressKey(address, sid)}`), limit: UNLOCKS_PER_LOG_AND_ADDRESS },
      { ref: attempts.doc(`address-${addressKey(address)}`), limit: UNLOCKS_PER_ADDRESS },
    );
  }
  return buckets;
}

/** How recently a full bucket must have been counted in to be full of guesses still being checked. */
const CHECKING_MS = 10_000;

/**
 * Each bucket's count this window (and when the window began), and how long
 * until a full one has room: the window's end, or only a moment if it was
 * counted in just now (right guesses among those are about to be given back).
 */
function counted(snaps: FirebaseFirestore.DocumentSnapshot[], limits: number[], now: number) {
  let wait = 0;
  const counts = snaps.map((snap, i) => {
    const since = (snap.get("since") as Timestamp | undefined)?.toMillis();
    if (since === undefined || now - since >= UNLOCK_WINDOW_MS) return { n: 0, since: now };
    const n = Number(snap.get("n")) || 0;
    if (n >= limits[i]) {
      const at = (snap.get("at") as Timestamp | undefined)?.toMillis() ?? since;
      const checking = now - at < CHECKING_MS;
      wait = Math.max(wait, checking ? UNLOCK_BUSY_SECONDS * 1000 : since + UNLOCK_WINDOW_MS - now);
    }
    return { n, since };
  });
  return { counts, wait: Math.ceil(wait / 1000) };
}

let warnedNoAddress = false;

/**
 * Counts a guess at log `id`'s password from `address`, unless the log, the
 * address, or the two together have had their fill of wrong guesses this
 * window: then it isn't counted, and the seconds until they may try again
 * are returned. Counted in a transaction, so guesses made at once can't get
 * past a limit; a right guess is given back with `giveBackUnlockAttempt`.
 */
export async function takeUnlockAttempt(
  id: string,
  address: string | null,
): Promise<{ wait: number } | { reservation: Reservation }> {
  if (!address && !warnedNoAddress) {
    console.warn("A request came without an address; guesses are only limited per log.");
    warnedNoAddress = true;
  }
  const buckets = unlockBuckets(id, address);
  const refs = buckets.map((b) => b.ref);
  const limits = buckets.map((b) => b.limit);
  // A flood of guesses meets full buckets: refused by a plain read, since a
  // transaction would only contend with the others.
  const before = counted(await firestore().getAll(...refs), limits, Date.now());
  if (before.wait > 0) return { wait: before.wait };
  try {
    return await firestore().runTransaction(async (tx) => {
      const now = Date.now();
      const { counts, wait } = counted(await tx.getAll(...refs), limits, now);
      if (wait > 0) return { wait };
      counts.forEach(({ n, since }, i) =>
        tx.set(refs[i], {
          n: n + 1,
          since: Timestamp.fromMillis(since),
          at: Timestamp.fromMillis(now),
          // For a TTL policy to clear them away (see README).
          expireAt: Timestamp.fromMillis(since + UNLOCK_WINDOW_MS),
        }),
      );
      return { reservation: counts.map(({ since }, i) => ({ ref: refs[i], since })) };
    });
  } catch (err) {
    // ABORTED: too many guesses at once to count them all.
    if ((err as { code?: number }).code === 10) return { wait: UNLOCK_BUSY_SECONDS };
    throw err;
  }
}

/**
 * Gives back a guess that was right, so only wrong ones use up the limits
 * (unless its window has since ended). If that fails, it stays counted.
 */
export async function giveBackUnlockAttempt(reservation: Reservation): Promise<void> {
  try {
    await firestore().runTransaction(async (tx) => {
      const snaps = await tx.getAll(...reservation.map((r) => r.ref));
      snaps.forEach((snap, i) => {
        const n = Number(snap.get("n")) || 0;
        const since = (snap.get("since") as Timestamp | undefined)?.toMillis();
        if (since === reservation[i].since && n > 0) tx.update(reservation[i].ref, { n: n - 1 });
      });
    });
  } catch (err) {
    console.error("Couldn't give back a right password guess; it counts as a wrong one.", err);
  }
}
