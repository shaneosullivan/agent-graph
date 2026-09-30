import {FieldPath, FieldValue, Timestamp} from "firebase-admin/firestore";

import {
  BYTES_PER_READ,
  CHUNKS_PER_QUERY,
  CHUNKS_PER_READ,
  SCRYPT_CHECKS_PER_LOG_AND_ADDRESS,
  SCRYPT_RUNS_PER_ADDRESS,
  type Source,
  UNLOCK_BUSY_SECONDS,
  UNLOCK_WINDOW_MS,
  UNLOCKS_PER_ADDRESS,
  UNLOCKS_PER_LOG,
  UNLOCKS_PER_LOG_AND_ADDRESS,
} from "./config";
import {addressKey, safeEqual} from "./crypto";
import {
  decryptChunk,
  encryptChunk,
  metaTag,
  openLogId,
  sealLogId,
  storageId,
} from "./encryption";
import {firestore} from "./firebase";

/**
 * How logs are kept in Firestore:
 *
 *   logs/{sid}                 { source, pw?, owner?, until?, lastAt?, createdAt, mac }
 *   logs/{sid}/chunks/{offset} { e: <encrypted JSON Lines, bytes>, n: <its length>, end: <offset + n>, t: <when written> }
 *   unlock-attempts/{bucket}   { n, since, at, expireAt }  (password guesses, and scrypt runs)
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
 * new document (and a read of the one it follows): nothing already stored is
 * rewritten, so the cost doesn't grow with the log. Readers page through chunks in id
 * order, and never read one twice, so a chunk never changes: a retry of the
 * same bytes is accepted, and different bytes at an offset already stored
 * are refused (checking costs a read, only then).
 *
 * A live share keeps only its last two keyframes' worth (see `trimLog`), so a
 * log's first chunk isn't always at offset 0.
 *
 * Each chunk starts where the one before it ends (see `appendChunk`), so
 * what a log stores is the span from its first chunk to its last, which is
 * what `MAX_LOG_BYTES` limits. (Logs made before this may have a `stored`
 * count; it's no longer kept.)
 *
 * An account's live share has `until`, when appends to it stop (`Unpaid`),
 * unless the account starts it again first (`setUntil`; lib/billing.ts).
 * It isn't in `mac`: it changes, and only says when to stop. It has
 * `lastAt` too, when events last came, to the minute (see `appendChunk`):
 * /admin counts the live shares active now by it (lib/analytics.ts).
 */

export type Meta = {
  source: Source;
  /** scrypt hash of the viewer password, if there is one. */
  pw?: string;
  /** The account whose live share it is, if it's one: only they can view it. */
  owner?: string;
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

/**
 * Creates log `id` with its first chunk, all in one commit; appends to it
 * stop at `until` (ms), if it's given.
 */
export async function createLog(
  id: string,
  meta: Omit<Meta, "createdAt">,
  text: string,
  until: number | null = null,
): Promise<void> {
  const log = logDoc(id);
  const batch = firestore().batch();
  // `create` fails if the id is already taken, rather than overwriting it.
  // (Firestore rejects undefined fields, so `pw` and `owner` are only set
  // when present.)
  batch.create(log, {
    source: meta.source,
    ...(meta.pw ? {pw: meta.pw} : {}),
    ...(meta.owner ? {owner: meta.owner} : {}),
    ...(until !== null ? {until: Timestamp.fromMillis(until)} : {}),
    ...(meta.owner ? {lastAt: Timestamp.now()} : {}),
    createdAt: Timestamp.now(),
    mac: metaTag(id, meta),
  });
  if (text) {
    const key = chunkKey(0);
    batch.set(log.collection("chunks").doc(key), chunkData(id, key, text));
  }
  try {
    await batch.commit();
  } catch (err) {
    // gRPC ALREADY_EXISTS
    if ((err as {code?: number}).code === 6) {
      throw new IdTaken(id);
    }
    throw err;
  }
}

/** A chunk's document: its text, encrypted; its length, and where it ends; and when it was written. */
function chunkData(id: string, key: string, text: string) {
  const n = Buffer.byteLength(text);
  return {
    e: encryptChunk(id, key, text),
    n,
    end: Number(key) + n,
    t: Timestamp.now(),
  };
}

export class ChunkTaken extends Error {}

/** The log isn't there (any more): one not in use is deleted (lib/cleanup.ts). */
export class LogGone extends Error {}

/** The chunk doesn't start where the log's last one ends. */
export class Misplaced extends Error {}

/** Appends to a live share stop once its account's time's up (lib/billing.ts). */
export class Unpaid extends Error {}

/** When appends to log `id` stop (ms), or null for never, from now on. */
export async function setUntil(
  id: string,
  until: number | null,
): Promise<void> {
  await logDoc(id).update({
    until: until === null ? FieldValue.delete() : Timestamp.fromMillis(until),
    // Carried on with: active again.
    lastAt: Timestamp.now(),
  });
}

/**
 * What a live share's `agent-graph watch-remote` last said of itself (see
 * `setAlive`): when (ms), and how many sessions it was watching.
 */
export type Alive = {at: number; sessions: number};

/**
 * A live share, as its account's /watch lists it (`sharesOf`): its log's
 * id, the computer it's shared from, when it last said it was running, how
 * many sessions it's watching, and their summaries (the viewer's session
 * list's, session id → summary: see the CLI's `timeline::summaries`).
 */
export type Share = {
  id: string;
  host: string;
  at: number;
  sessions: number;
  summary: Record<string, unknown>;
};

/** Where a share's summary is sealed (as its chunks are, with its own key). */
const SUMMARY_CHUNK = "alive-summary";

/**
 * Notes that log `id` (owned by account `owner`)'s `agent-graph
 * watch-remote` is still running, on computer `host`, watching `sessions`
 * sessions, whose summaries are `summary`: it says so every minute or so.
 * The computer's name and the summaries are encrypted with the log's key;
 * its id's sealed for its owner, so their shares can be listed
 * (`sharesOf`). One write. `LogGone` if the log isn't there.
 */
export async function setAlive(
  id: string,
  owner: string,
  sessions: number,
  host = "",
  summary: Record<string, unknown> = {},
): Promise<void> {
  try {
    await logDoc(id).update({
      alive: {
        at: Timestamp.now(),
        sessions,
        log: sealLogId(owner, id),
        summary: encryptChunk(
          id,
          SUMMARY_CHUNK,
          JSON.stringify({host, summary}),
        ),
      },
    });
  } catch (err) {
    // (Firestore's NOT_FOUND: there's no such log.)
    if ((err as {code?: unknown}).code === 5) {
      throw new LogGone();
    }
    throw err;
  }
}

/** What log `id`'s `watch-remote` last said of itself, if it has (see `setAlive`). */
export async function getAlive(id: string): Promise<Alive | null> {
  const snap = await logDoc(id).get();
  if (!snap.exists || snap.get("deleting")) {
    return null;
  }
  const alive = snap.get("alive") as
    {at?: unknown; sessions?: unknown} | undefined;
  if (!(alive?.at instanceof Timestamp) || typeof alive.sessions !== "number") {
    return null;
  }
  return {at: alive.at.toMillis(), sessions: alive.sessions};
}

/**
 * Account `uid`'s live shares that have said they're running (see
 * `setAlive`), most recent first: one per computer or cloud instance
 * sharing. A share whose summary can't be read (sealed for another
 * account, or altered) is left out.
 */
export async function sharesOf(uid: string): Promise<Array<Share>> {
  const owned = await logs().where("owner", "==", uid).get();
  const shares: Array<Share> = [];
  for (const doc of owned.docs) {
    if (doc.get("deleting")) {
      continue;
    }
    const alive = doc.get("alive") as
      | {at?: unknown; sessions?: unknown; log?: unknown; summary?: unknown}
      | undefined;
    if (
      !(alive?.at instanceof Timestamp) ||
      !(alive.log instanceof Uint8Array) ||
      !(alive.summary instanceof Uint8Array)
    ) {
      continue;
    }
    const id = openLogId(uid, alive.log);
    if (!id) {
      continue;
    }
    try {
      const {host, summary} = JSON.parse(
        decryptChunk(id, SUMMARY_CHUNK, alive.summary),
      ) as {host?: unknown; summary?: unknown};
      shares.push({
        id,
        host: typeof host === "string" ? host : "",
        at: alive.at.toMillis(),
        sessions: typeof alive.sessions === "number" ? alive.sessions : 0,
        summary:
          summary && typeof summary === "object"
            ? (summary as Record<string, unknown>)
            : {},
      });
    } catch {
      // Not readable: left out.
    }
  }
  return shares.sort((a, b) => b.at - a.at);
}

/** How often, at most, a live share's `lastAt` is brought up to date. */
const LAST_AT_EVERY_MS = 60 * 1000;

/**
 * Whether log `id` is there to add to: stored, and not being deleted
 * (lib/cleanup.ts marks one first), or, until it's moved, stored as it was
 * before storage ids (scripts/migrate-storage-ids.mjs). `get` reads a
 * document (in a transaction, say).
 */
async function there(
  id: string,
  get: (
    ref: FirebaseFirestore.DocumentReference,
  ) => Promise<FirebaseFirestore.DocumentSnapshot>,
): Promise<boolean> {
  return (await where(id, get)) !== null;
}

/**
 * Log `id`'s document under its storage id, as `get` read it, if the log's
 * there (see `there`); `undefined` if it's there only as it was stored
 * before storage ids (it has no count of what it stores), null if it isn't.
 */
async function where(
  id: string,
  get: (
    ref: FirebaseFirestore.DocumentReference,
  ) => Promise<FirebaseFirestore.DocumentSnapshot>,
): Promise<FirebaseFirestore.DocumentSnapshot | undefined | null> {
  const log = await get(logDoc(id));
  if (log.exists) {
    return log.get("deleting") ? null : log;
  }
  const old = await get(logs().doc(id));
  return old.exists && !old.get("deleting") ? undefined : null;
}

/**
 * Encrypts and stores one chunk at `offset`, if the log's still there
 * (`LogGone` if not), its `until` hasn't passed (`Unpaid` if it has), and
 * it follows on from what's there (`Misplaced` if not): a chunk ends where it starts, or, if none does, it's the log's
 * first (at 0, with nothing stored yet) or a retry of the first one kept.
 * So chunks never overlap, nor leave gaps but where a log's start was
 * trimmed, and what a log stores is the span from its first chunk to its
 * last, which the append route keeps within `MAX_LOG_BYTES`. One write, of
 * the chunk, and two reads (the log's metadata, and the chunk it follows
 * on from), unless a chunk is already stored there: then it's read, and
 * anything but the same bytes (or one that can't be decrypted) is refused
 * with `ChunkTaken`.
 *
 * (Chunks stored before they said where they end don't: a log whose first
 * chunk is one of those isn't checked, until it's trimmed past them.)
 */
export async function appendChunk(
  id: string,
  offset: number,
  text: string,
): Promise<void> {
  const key = chunkKey(offset);
  const chunks = logDoc(id).collection("chunks");
  const doc = chunks.doc(key);
  try {
    // Only while the log's there (read with it, so a deletion as it's
    // written makes it try again, and fail): otherwise what's sent after
    // it's deleted would be kept for good.
    await firestore().runTransaction(async tx => {
      const found = await where(id, ref => tx.get(ref));
      if (found === null) {
        throw new LogGone(id);
      }
      const until = found?.get("until") as Timestamp | undefined;
      if (until && until.toMillis() < Date.now()) {
        throw new Unpaid(id);
      }
      // A live share's when events last came, to the minute: a write only
      // once a minute, not with every chunk. (Written below, as a
      // transaction reads everything before it writes.)
      const lastAt = found?.get("lastAt") as Timestamp | undefined;
      const touch =
        found &&
        found.get("owner") &&
        (!lastAt || Date.now() - lastAt.toMillis() > LAST_AT_EVERY_MS)
          ? found.ref
          : null;
      // (One still stored as it was before storage ids has chunks from
      // before that aren't here to follow on from.)
      if (found) {
        const ends =
          offset > 0
            ? await tx.get(chunks.where("end", "==", offset).limit(1).select())
            : null;
        if (!ends || ends.empty) {
          const [first] = (
            await tx.get(
              chunks.orderBy(FieldPath.documentId()).limit(1).select("end"),
            )
          ).docs;
          // With nothing stored yet, it goes at 0. Else it's the first
          // kept, again (a retry: storing it, below, fails, and it's
          // compared), or it doesn't follow on.
          const follows = first
            ? first.id === key || (offset > 0 && first.get("end") === undefined)
            : offset === 0;
          if (!follows) {
            throw new Misplaced(key);
          }
        }
      }
      // When it was written: a log with none newer than a week is deleted.
      tx.create(doc, chunkData(id, key, text));
      if (touch) {
        tx.update(touch, {lastAt: Timestamp.now()});
      }
    });
  } catch (err) {
    // gRPC ALREADY_EXISTS
    if ((err as {code?: number}).code !== 6) {
      throw err;
    }
    const stored = (await doc.get()).get("e");
    if (!stored || !holds(id, key, stored, text)) {
      throw new ChunkTaken(key);
    }
  }
}

/**
 * Whether a stored chunk holds `text`. One that can't be decrypted doesn't:
 * refusing it tells the client to stop, where an error would have it retry
 * for good.
 */
function holds(
  id: string,
  key: string,
  stored: Uint8Array,
  text: string,
): boolean {
  try {
    return decryptChunk(id, key, stored) === text;
  } catch {
    // Named by where it's stored: the log's id is what lets people read it.
    console.error(
      `Chunk logs/${storageId(id)}/chunks/${key} couldn't be decrypted; refused the append there.`,
    );
    return false;
  }
}

/** Chunks deleted in one transaction, which Firestore applies all at once. */
const TRIM_BATCH = 500;

/**
 * Deletes log `id`'s chunks that start before offset `before`: a live share
 * keeps only its last two keyframes' worth, and when it sends a keyframe,
 * asks for everything before the previous one to go. They're deleted
 * newest first, a batch at a time (each all at once), so a reader never
 * starts partway through what's being deleted; one that's reading across it
 * sees a gap, and starts again (site-source.js). Each batch is a
 * transaction, with the log (so none go from a log being deleted). Returns
 * how many went (those still there: two trims at once count each once).
 */
export async function trimLog(id: string, before: number): Promise<number> {
  if (!(await there(id, ref => ref.get()))) {
    throw new LogGone(id);
  }
  // Their keys, oldest first (Firestore can't scan keys the other way).
  const chunks = logDoc(id).collection("chunks");
  const older = chunks
    .orderBy(FieldPath.documentId())
    .endBefore(chunkKey(before))
    .select();
  const keys: Array<string> = [];
  for (;;) {
    const page = keys.length ? older.startAfter(keys[keys.length - 1]) : older;
    const snap = await page.limit(TRIM_BATCH).get();
    keys.push(...snap.docs.map(doc => doc.id));
    if (snap.size < TRIM_BATCH) {
      break;
    }
  }
  let deleted = 0;
  for (let end = keys.length; end > 0; end -= TRIM_BATCH) {
    const refs = keys
      .slice(Math.max(0, end - TRIM_BATCH), end)
      .map(key => chunks.doc(key));
    deleted += await firestore().runTransaction(async tx => {
      const found = await where(id, ref => tx.get(ref));
      if (found === null) {
        throw new LogGone(id);
      }
      const snaps = await tx.getAll(...refs, {fieldMask: []});
      const stored = snaps.filter(snap => snap.exists);
      for (const snap of stored) {
        tx.delete(snap.ref);
      }
      return stored.length;
    });
  }
  return deleted;
}

/** Where log `id`'s first chunk (the oldest kept) starts, or null if it has none. */
export async function firstChunkOffset(id: string): Promise<number | null> {
  const snap = await logDoc(id)
    .collection("chunks")
    .orderBy(FieldPath.documentId())
    .limit(1)
    .select()
    .get();
  return snap.empty ? null : Number(snap.docs[0].id);
}

// Metadata never changes after creation, so each server instance keeps what
// it has read. That saves a read on every viewer poll.
// Metadata doesn't change, but can go (a log not in use is deleted), so
// what's kept is read again after a while, and forgotten by an instance
// that deletes it.
const metaCache = new Map<string, {meta: Meta; at: number}>();
export const META_CACHE_MS = 10 * 60 * 1000;
const META_CACHE_LIMIT = 5000;

/** Forgets log `sid`'s cached metadata: it's been deleted. */
export function forgetMeta(sid: string): void {
  metaCache.delete(sid);
}

/** Log `id`'s metadata; null if there's no such log, or if it was changed. */
export async function getMeta(
  id: string,
  now = Date.now(),
): Promise<Meta | null> {
  const cached = metaCache.get(storageId(id));
  if (cached && now - cached.at <= META_CACHE_MS) {
    return cached.meta;
  }
  const snap = await logDoc(id).get();
  let meta: Meta | null = null;
  // (One being deleted is gone.)
  if (snap.exists && !snap.get("deleting")) {
    const {source, pw, owner, createdAt, mac} = snap.data() as Meta & {
      mac?: unknown;
    };
    if (
      typeof mac === "string" &&
      safeEqual(mac, metaTag(id, {source, pw, owner}))
    ) {
      meta = {
        source,
        ...(pw ? {pw} : {}),
        ...(owner ? {owner} : {}),
        createdAt,
      };
    } else {
      // Named by where it's stored: the log's id is what lets people read it.
      console.error(
        `The metadata of logs/${storageId(id)} doesn't match its tag; treated as missing.`,
      );
    }
  }
  // Don't cache "not found": the log may be created a moment later.
  if (!meta) {
    metaCache.delete(storageId(id));
  }
  if (meta) {
    if (metaCache.size >= META_CACHE_LIMIT) {
      metaCache.delete(metaCache.keys().next().value!);
    }
    metaCache.set(storageId(id), {meta, at: now});
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
): Promise<{
  text: string;
  first: string | null;
  last: string | null;
  more: boolean;
}> {
  const chunks = logDoc(id)
    .collection("chunks")
    .orderBy(FieldPath.documentId());
  const texts: Array<string> = [];
  let bytes = 0;
  let first: string | null = null;
  let last: string | null = null;
  // Where the next chunk should start.
  let next: number | null = null;
  while (texts.length < CHUNKS_PER_READ) {
    const want = Math.min(CHUNKS_PER_QUERY, CHUNKS_PER_READ - texts.length);
    const cursor: string = last ?? after;
    const snap = await (cursor ? chunks.startAfter(cursor) : chunks)
      .limit(want)
      .get();
    for (const doc of snap.docs) {
      if (next !== null && Number(doc.id) !== next) {
        return {text: texts.join(""), first, last, more: true};
      }
      const text = decryptChunk(id, doc.id, doc.get("e"));
      texts.push(text);
      first ??= doc.id;
      last = doc.id;
      next = Number(doc.id) + Buffer.byteLength(text);
      bytes += Buffer.byteLength(text);
      if (bytes >= BYTES_PER_READ) {
        return {text: texts.join(""), first, last, more: true};
      }
    }
    if (snap.size < want) {
      return {text: texts.join(""), first, last, more: false};
    }
  }
  return {text: texts.join(""), first, last, more: true};
}

/** A guess counted by `takeUnlockAttempt`, to give back if it was right. */
export type Reservation = Array<{
  ref: FirebaseFirestore.DocumentReference;
  since: number;
}>;

/**
 * A count, per window, of guesses (or scrypt runs), up to `limit`. A guess
 * that turns out right is given back to all but those that are `kept`.
 */
type Bucket = {
  ref: FirebaseFirestore.DocumentReference;
  limit: number;
  kept?: boolean;
};

/** The bucket counting `address`'s scrypt runs, whatever they're for. */
function scryptBucket(address: string): Bucket {
  const ref = firestore()
    .collection("unlock-attempts")
    .doc(`scrypt-${addressKey(address)}`);
  return {ref, limit: SCRYPT_RUNS_PER_ADDRESS, kept: true};
}

/** The buckets a guess at log `id` from `address` counts against. */
function unlockBuckets(id: string, address: string | null): Array<Bucket> {
  const attempts = firestore().collection("unlock-attempts");
  const sid = storageId(id);
  // All keyed: the documents name neither the log nor the address.
  const buckets: Array<Bucket> = [
    {ref: attempts.doc(`log-${sid}`), limit: UNLOCKS_PER_LOG},
  ];
  if (address) {
    buckets.push(
      {
        ref: attempts.doc(`pair-${addressKey(address, sid)}`),
        limit: UNLOCKS_PER_LOG_AND_ADDRESS,
      },
      {
        ref: attempts.doc(`address-${addressKey(address)}`),
        limit: UNLOCKS_PER_ADDRESS,
      },
      // Checking a guess runs scrypt, right or wrong: counted at the log,
      // and in all.
      {
        ref: attempts.doc(`checks-${addressKey(address, sid)}`),
        limit: SCRYPT_CHECKS_PER_LOG_AND_ADDRESS,
        kept: true,
      },
      scryptBucket(address),
    );
  }
  return buckets;
}

/** How recently a full bucket must have been counted in to be full of guesses still being checked. */
const CHECKING_MS = 10_000;

/**
 * Each bucket's count this window (and when the window began), and how long
 * until a full one has room: the window's end, or only a moment if it was
 * counted in just now (right guesses among those are about to be given
 * back, unless it's `kept`).
 */
function counted(
  snaps: Array<FirebaseFirestore.DocumentSnapshot>,
  buckets: Array<Bucket>,
  now: number,
) {
  let wait = 0;
  const counts = snaps.map((snap, i) => {
    const since = (snap.get("since") as Timestamp | undefined)?.toMillis();
    if (since === undefined || now - since >= UNLOCK_WINDOW_MS) {
      return {n: 0, since: now};
    }
    const n = Number(snap.get("n")) || 0;
    if (n >= buckets[i].limit) {
      const at = (snap.get("at") as Timestamp | undefined)?.toMillis() ?? since;
      const checking = !buckets[i].kept && now - at < CHECKING_MS;
      wait = Math.max(
        wait,
        checking ? UNLOCK_BUSY_SECONDS * 1000 : since + UNLOCK_WINDOW_MS - now,
      );
    }
    return {n, since};
  });
  return {counts, wait: Math.ceil(wait / 1000)};
}

let warnedNoAddress = false;

/**
 * Counts one in each of `buckets`, unless any has had its fill this window:
 * then none is, and the seconds until there's room are returned. Counted in
 * a transaction, so ones made at once can't get past a limit. Returns each
 * bucket's window, for giving it back.
 */
async function take(
  buckets: Array<Bucket>,
): Promise<{wait: number} | {since: Array<number>}> {
  const refs = buckets.map(b => b.ref);
  // A flood meets full buckets: refused by a plain read, since a
  // transaction would only contend with the others.
  const before = counted(
    await firestore().getAll(...refs),
    buckets,
    Date.now(),
  );
  if (before.wait > 0) {
    return {wait: before.wait};
  }
  try {
    return await firestore().runTransaction(async tx => {
      const now = Date.now();
      const {counts, wait} = counted(await tx.getAll(...refs), buckets, now);
      if (wait > 0) {
        return {wait};
      }
      counts.forEach(({n, since}, i) =>
        tx.set(refs[i], {
          n: n + 1,
          since: Timestamp.fromMillis(since),
          at: Timestamp.fromMillis(now),
          // For a TTL policy to clear them away (see README).
          expireAt: Timestamp.fromMillis(since + UNLOCK_WINDOW_MS),
        }),
      );
      return {since: counts.map(({since}) => since)};
    });
  } catch (err) {
    // ABORTED: too many at once to count them all.
    if ((err as {code?: number}).code === 10) {
      return {wait: UNLOCK_BUSY_SECONDS};
    }
    throw err;
  }
}

/**
 * Counts a guess at log `id`'s password from `address`, unless the log, the
 * address, or the two together have had their fill of wrong guesses this
 * window, or the address its fill of scrypt runs (at this log, or in all):
 * then it isn't counted, and the seconds until they may try again are
 * returned. Guesses made at once can't get past a limit; a right guess is
 * given back with `giveBackUnlockAttempt` (but for its scrypt runs).
 */
export async function takeUnlockAttempt(
  id: string,
  address: string | null,
): Promise<{wait: number} | {reservation: Reservation}> {
  if (!address && !warnedNoAddress) {
    console.warn(
      "A request came without an address; guesses are only limited per log.",
    );
    warnedNoAddress = true;
  }
  const buckets = unlockBuckets(id, address);
  const taken = await take(buckets);
  if ("wait" in taken) {
    return taken;
  }
  return {
    reservation: buckets.flatMap(({ref, kept}, i) =>
      kept ? [] : [{ref, since: taken.since[i]}],
    ),
  };
}

/**
 * Counts a scrypt run from `address` that isn't checking a guess (hashing a
 * new log's password), unless it's had its fill this window: then the
 * seconds until it may run another are returned. Not limited without an
 * address.
 */
export async function takeScryptRun(
  address: string | null,
): Promise<{wait: number} | Record<string, never>> {
  if (!address) {
    return {};
  }
  const taken = await take([scryptBucket(address)]);
  return "wait" in taken ? taken : {};
}

/**
 * Gives back a guess that was right, so only wrong ones use up the limits
 * (unless its window has since ended). If that fails, it stays counted.
 */
export async function giveBackUnlockAttempt(
  reservation: Reservation,
): Promise<void> {
  try {
    await firestore().runTransaction(async tx => {
      const snaps = await tx.getAll(...reservation.map(r => r.ref));
      snaps.forEach((snap, i) => {
        const n = Number(snap.get("n")) || 0;
        const since = (snap.get("since") as Timestamp | undefined)?.toMillis();
        if (since === reservation[i].since && n > 0) {
          tx.update(reservation[i].ref, {n: n - 1});
        }
      });
    });
  } catch (err) {
    console.error(
      "Couldn't give back a right password guess; it counts as a wrong one.",
      err,
    );
  }
}
