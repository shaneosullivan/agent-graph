import {randomInt} from "node:crypto";

import {FieldValue, Timestamp} from "firebase-admin/firestore";

import {
  analyticsDisabled,
  type Counter,
  COUNTERS,
  firstDayKept,
  periods,
} from "./analytics-core";
import {firestore} from "./firebase";

/**
 * Counting how the site's used (what's counted: lib/analytics-core.ts), for
 * /admin. Nothing's counted with ANALYTICS_DISABLED=true.
 *
 *   analytics-days/{day}-{shard}      { period: "2026-09-29", c: { <counter>: n } }
 *   analytics-months/{month}-{shard}  { period: "2026-09", c: { <counter>: n } }
 *
 * A count goes to one of `SHARDS` documents for its period, at random, and
 * they're added up when read: Firestore takes about one write a second to
 * a document, so a busy day's counts are spread out. Months are counted as
 * they happen, not added up from days, so a day's counts can go once
 * they're a year old (`deleteOldDays`, run daily by the cron: app/api/cron/analytics)
 * and its month's are kept.
 *
 * Browsers report through POST /api/analytics (lib/analytics-client.ts);
 * the site counts what it does itself with `count`.
 */

/** Documents per period that counts are spread over. */
export const SHARDS = 8;

const days = () => firestore().collection("analytics-days");
const months = () => firestore().collection("analytics-months");

/** Adds one to each of `day`'s counters for today, and `month`'s for this month. */
export async function countEach(
  day: ReadonlyArray<Counter>,
  month: ReadonlyArray<Counter>,
  at = new Date(),
): Promise<void> {
  if (analyticsDisabled() || (!day.length && !month.length)) {
    return;
  }
  const {day: d, month: m} = periods(at);
  const shard = randomInt(SHARDS);
  const add = (counters: ReadonlyArray<Counter>) =>
    Object.fromEntries(counters.map(c => [c, FieldValue.increment(1)]));
  const batch = firestore().batch();
  if (day.length) {
    batch.set(
      days().doc(`${d}-${shard}`),
      {period: d, c: add(day)},
      {merge: true},
    );
  }
  if (month.length) {
    batch.set(
      months().doc(`${m}-${shard}`),
      {period: m, c: add(month)},
      {merge: true},
    );
  }
  await batch.commit();
}

/**
 * Counts `counter` once, for today and this month. Never fails what it's
 * counted for: a count that can't be stored is only logged.
 */
export async function count(counter: Counter): Promise<void> {
  await countEach([counter], [counter]).catch(err =>
    console.error(`Counting ${counter}:`, err),
  );
}

export type Counts = Record<Counter, number>;
export type Period = {period: string; counts: Counts};

const zero = (): Counts =>
  Object.fromEntries(COUNTERS.map(c => [c, 0])) as Counts;

/** Each of `periodsWanted`'s counts, in that order, 0 where nothing was counted. */
async function read(
  collection: FirebaseFirestore.CollectionReference,
  periodsWanted: Array<string>,
): Promise<Array<Period>> {
  const byPeriod = new Map(periodsWanted.map(p => [p, zero()]));
  if (periodsWanted.length) {
    const snap = await collection
      .where("period", ">=", periodsWanted[0])
      .where("period", "<=", periodsWanted[periodsWanted.length - 1])
      .get();
    for (const doc of snap.docs) {
      const counts = byPeriod.get(doc.get("period"));
      const c = (doc.get("c") ?? {}) as Record<string, unknown>;
      if (!counts) {
        continue;
      }
      for (const counter of COUNTERS) {
        const n = c[counter];
        if (typeof n === "number") {
          counts[counter] += n;
        }
      }
    }
  }
  return periodsWanted.map(period => ({period, counts: byPeriod.get(period)!}));
}

/** The last `n` days' counts, to today, oldest first. */
export function lastDays(n: number, now = new Date()): Promise<Array<Period>> {
  const wanted = Array.from({length: n}, (_, i) => {
    const at = new Date(now);
    at.setUTCDate(at.getUTCDate() - (n - 1 - i));
    return periods(at).day;
  });
  return read(days(), wanted);
}

/** The last `n` months' counts, to this month, oldest first. */
export function lastMonths(
  n: number,
  now = new Date(),
): Promise<Array<Period>> {
  const wanted = Array.from({length: n}, (_, i) => {
    const at = new Date(
      Date.UTC(now.getUTCFullYear(), now.getUTCMonth() - (n - 1 - i), 1),
    );
    return periods(at).month;
  });
  return read(months(), wanted);
}

/** How recently a live share must have sent events to count as active. */
export const ACTIVE_MS = 5 * 60 * 1000;

/**
 * Live shares active now: sent events in the last `ACTIVE_MS` (each
 * records when it last did: lib/store.ts, `lastAt`). A watch-remote with
 * nothing new to send for that long isn't counted.
 */
export async function activeWatches(now = Date.now()): Promise<number> {
  const snap = await firestore()
    .collection("logs")
    .where("lastAt", ">=", Timestamp.fromMillis(now - ACTIVE_MS))
    .count()
    .get();
  return snap.data().count;
}

/**
 * Deletes days' counts from before a year ago (their months' are kept).
 * How many documents it deleted.
 */
export async function deleteOldDays(now = new Date()): Promise<number> {
  const before = firstDayKept(now);
  let deleted = 0;
  for (;;) {
    const snap = await days().where("period", "<", before).limit(400).get();
    if (snap.empty) {
      return deleted;
    }
    const batch = firestore().batch();
    for (const doc of snap.docs) {
      batch.delete(doc.ref);
    }
    await batch.commit();
    deleted += snap.size;
  }
}
