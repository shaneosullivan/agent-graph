import {
  FieldPath,
  FieldValue,
  type QueryDocumentSnapshot,
  Timestamp,
} from "firebase-admin/firestore";

import {ID_PATTERN} from "./config";
import {safeEqual} from "./crypto";
import {storageId} from "./encryption";
import {firestore} from "./firebase";
import {forgetMeta} from "./store";

/**
 * Deleting logs that are no longer in use: a log that has had no event (no
 * chunk written) for `LOG_IDLE_MS` is deleted, by a daily cron
 * (app/api/cron/cleanup, vercel.json).
 *
 * Each chunk records when it was written (`t`, set in the write that stores
 * it, so an append is still one write), and a log's last event is its
 * newest chunk's. Chunks stored before that was recorded have no `t`: a log
 * with none that has one counts as active when the cron first ran
 * (`cleanup/state.since`), so none is deleted for at least `LOG_IDLE_MS`
 * after the change.
 */

/**
 * Whether `req` is Vercel Cron's, which sends `Authorization: Bearer
 * <CRON_SECRET>`. Without the secret set, nothing is.
 */
export function cronAllowed(
  req: Request,
  secret = process.env.CRON_SECRET,
): boolean {
  return (
    !!secret && safeEqual(req.headers.get("authorization"), `Bearer ${secret}`)
  );
}

/** How long a log can go without an event before it's deleted. */
export const LOG_IDLE_MS = 7 * 24 * 60 * 60 * 1000;

/** Logs looked at per query, and at once. */
const PAGE = 100;
const AT_ONCE = 10;

const state = () => firestore().collection("cleanup").doc("state");

/**
 * Deletes the logs idle for more than `LOG_IDLE_MS` as of `now`, looking at
 * logs in id order, a page at a time, until `budgetMs` has passed (a page
 * at least, so every run gets somewhere); the next run carries on from there
 * (`cleanup/state.cursor`). Returns how many it looked at and deleted, and
 * whether it got through them all.
 */
export async function deleteIdleLogs(opts: {
  now?: number;
  budgetMs: number;
  /** Logs per query. */
  page?: number;
}): Promise<{checked: number; deleted: number; done: boolean}> {
  const size = opts.page ?? PAGE;
  const started = Date.now();
  const now = opts.now ?? Date.now();
  const saved = await state().get();
  const since: number = saved.get("since")?.toMillis() ?? now;
  if (!saved.exists || saved.get("since") === undefined) {
    await state().set({since: Timestamp.fromMillis(since)}, {merge: true});
  }
  let cursor: string | undefined = saved.get("cursor") ?? undefined;
  const logs = firestore().collection("logs").orderBy(FieldPath.documentId());
  let checked = 0;
  let deleted = 0;
  for (;;) {
    const page = await (cursor ? logs.startAfter(cursor) : logs)
      .limit(size)
      .get();
    for (let i = 0; i < page.docs.length; i += AT_ONCE) {
      const group = page.docs.slice(i, i + AT_ONCE);
      const idle = await Promise.all(group.map(log => isIdle(log, since, now)));
      for (const [j, gone] of idle.entries()) {
        checked++;
        if (gone && (await deleteIfIdle(group[j].id, since, now))) {
          deleted++;
        }
      }
      cursor = group[group.length - 1].id;
      // Time's checked as it goes: deleting a log can take a while.
      if (
        Date.now() - started > opts.budgetMs &&
        (i + AT_ONCE < page.docs.length || page.size === size)
      ) {
        await state().set({cursor}, {merge: true});
        return {checked, deleted, done: false};
      }
    }
    if (page.size < size) {
      await state().set({cursor: FieldValue.delete()}, {merge: true});
      return {checked, deleted, done: true};
    }
  }
}

/**
 * Deletes log document `key` if it's idle (see `isIdle`), judged again in a
 * transaction with its newest chunk, as it's marked for deletion: so an
 * append that lands as it's judged (store.ts's appendChunk, which reads the
 * log in its own) either comes first, and the log is kept, or finds it
 * marked, and is refused. Then its chunks go, and the log last, so a
 * deletion that stops partway is finished by the next run (it's still
 * there, marked); and the count of guesses at its password. Returns whether
 * it was deleted.
 *
 * A log stored as it was before storage ids (`key` is its id; see
 * scripts/migrate-storage-ids.mjs) is judged and deleted the same way,
 * with what's been added to it since, which is stored under its storage
 * id; and its copy there, if the migration's made one, which is left until
 * then (it's marked `oldCopy` until the old one's deleted): deleted on its
 * own, the old one would still take appends, which no one could read. A
 * copy made before that mark is a log of its own.
 */
export async function deleteIfIdle(
  key: string,
  since: number,
  now: number,
): Promise<boolean> {
  return deleteLogDoc(key, (log, newest) => idleSince(log, newest, since, now));
}

/**
 * Deletes every log owned by account `uid` (its live shares), now, idle or
 * not: its account's being deleted. How many it deleted.
 */
export async function deleteLogsOf(uid: string): Promise<number> {
  const owned = await firestore()
    .collection("logs")
    .where("owner", "==", uid)
    .get();
  let deleted = 0;
  for (const log of owned.docs) {
    if (await deleteLogDoc(log.id, () => true)) {
      deleted++;
    }
  }
  return deleted;
}

/**
 * Deletes log document `key`, if `due` says so (given the log, and its
 * newest chunk), judged in the transaction that marks it for deletion: see
 * `deleteIfIdle`.
 */
async function deleteLogDoc(
  key: string,
  due: (
    log: FirebaseFirestore.DocumentSnapshot,
    newest: FirebaseFirestore.QuerySnapshot,
  ) => boolean,
): Promise<boolean> {
  const {ref, added, sid} = where(key);
  const old = !added.isEqual(ref);
  const marked = await firestore().runTransaction(async tx => {
    const log = await tx.get(ref);
    if (!log.exists) {
      return false;
    }
    if (log.get("deleting")) {
      return true;
    }
    // A copy goes with its old copy (below), which appends accept too.
    if (log.get("oldCopy")) {
      return false;
    }
    const copy = old ? await tx.get(added) : null;
    const newest = await tx.get(newestChunk(added));
    if (!due(log, newest)) {
      return false;
    }
    tx.update(ref, {deleting: true});
    if (copy?.get("oldCopy")) {
      tx.update(added, {deleting: true});
    }
    return true;
  });
  if (!marked) {
    return false;
  }
  forgetMeta(sid);
  await firestore().recursiveDelete(ref.collection("chunks"));
  if (old) {
    const copy = await added.get();
    if (!copy.exists) {
      await firestore().recursiveDelete(added.collection("chunks"));
    } else if (copy.get("oldCopy")) {
      await firestore().recursiveDelete(added);
    }
  }
  await ref.delete();
  await firestore().collection("unlock-attempts").doc(`log-${sid}`).delete();
  return true;
}

/**
 * Log document `key`'s reference, where what's added to it is stored (its
 * chunks with times), and its storage id.
 */
function where(key: string) {
  const logs = firestore().collection("logs");
  const sid = ID_PATTERN.test(key) ? storageId(key) : key;
  return {ref: logs.doc(key), added: logs.doc(sid), sid};
}

/** Whether `log` has had no event for `LOG_IDLE_MS` as of `now`, or is being deleted. */
async function isIdle(
  log: QueryDocumentSnapshot,
  since: number,
  now: number,
): Promise<boolean> {
  if (log.get("deleting")) {
    return true;
  }
  return idleSince(
    log,
    await newestChunk(where(log.id).added).get(),
    since,
    now,
  );
}

const newestChunk = (log: FirebaseFirestore.DocumentReference) =>
  log.collection("chunks").orderBy("t", "desc").limit(1).select("t");

/**
 * Whether a log has had no event for `LOG_IDLE_MS` as of `now`: its newest
 * chunk's time (`newest`), or if none has one, when it was created or when
 * times started being recorded (`since`), whichever is later.
 */
function idleSince(
  log: FirebaseFirestore.DocumentSnapshot,
  newest: FirebaseFirestore.QuerySnapshot,
  since: number,
  now: number,
): boolean {
  const last = newest.empty
    ? Math.max(log.get("createdAt")?.toMillis() ?? 0, since)
    : (newest.docs[0].get("t") as Timestamp).toMillis();
  return now - last > LOG_IDLE_MS;
}
