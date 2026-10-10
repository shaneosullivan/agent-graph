// API keys (site/openapi.json, "Authentication"): read-only keys to the
// graph API, made and revoked on the account page.
//
//   api-keys/{sha256}  { uid, kind, graphs, name, last4, version, createdAt, usedAt,
//                        browser?, expireAt? }
//
// Stored by their SHA-256, as the CLI's tokens are (lib/accounts.ts): the
// database alone can't be used to read anyone's graphs. A secret key
// (`ag_sk_live_…`) reads every graph its account owns; a restricted one
// (`ag_rk_live_…`), only those it was made for (`graphs`, log ids). Each is
// pinned to the API version current when it was made.
//
// A browser key (`browser: true`) is made for a browser that's logged in,
// for the API showcase to read its account's graphs with
// (app/api/showcase/key): a secret key that stops working an hour after
// it's made (`expireAt`, which a Firestore TTL policy can clear away). It
// isn't listed on the account page, nor counted against MOST_API_KEYS.

import {createHash, randomBytes} from "node:crypto";

import {Timestamp} from "firebase-admin/firestore";

import {API_VERSION} from "./errors";
import {firestore} from "../firebase";

export type KeyKind = "secret" | "restricted";

/** A key, as a request made with it is checked against. */
export type Key = {
  id: string;
  uid: string;
  kind: KeyKind;
  /** For a restricted key, the logs it can read. */
  graphs: Array<string> | null;
  version: string;
};

/** A key, as the account page lists it. */
export type KeyInfo = Omit<Key, "uid"> & {
  name: string;
  /** How it's shown: its prefix, and its last four characters. */
  shown: string;
  createdAt: number;
  usedAt: number | null;
};

/** The most keys an account can have at once. */
export const MOST_API_KEYS = 20;

/** The most graphs a restricted key can be made for. */
export const MOST_KEY_GRAPHS = 50;

export const KEY_PATTERN = /^ag_(sk|rk)_live_[A-Za-z0-9]{32}$/;

/** How long a browser key works for. */
export const BROWSER_KEY_MS = 60 * 60 * 1000;

/** The most browser keys an account keeps at once: the oldest go first. */
const MOST_BROWSER_KEYS = 20;

/** How often, at most, a key's `usedAt` is brought up to date. */
const USED_AT_EVERY_MS = 24 * 60 * 60 * 1000;

const keys = () => firestore().collection("api-keys");

const hash = (value: string) =>
  createHash("sha256").update(value).digest("hex");

const BASE62 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

/** `n` random base62 characters, each equally likely. */
function random62(n: number): string {
  let out = "";
  while (out.length < n) {
    for (const byte of randomBytes(n * 2)) {
      // 248 = 62 * 4: rejecting larger bytes keeps every character equally likely.
      if (byte < 248 && out.length < n) {
        out += BASE62[byte % 62];
      }
    }
  }
  return out;
}

/** A key as it's shown once made: `ag_sk_live_****abcd`. */
export function shownKey(key: string): string {
  return `${key.slice(0, 11)}****${key.slice(-4)}`;
}

/**
 * A new key for account `uid`: the key itself, shown this once, and what's
 * kept of it. Null if the account has `MOST_API_KEYS` already.
 */
export async function newKey(
  uid: string,
  {
    name,
    kind,
    graphs,
  }: {name: string; kind: KeyKind; graphs: Array<string> | null},
): Promise<{key: string; info: KeyInfo} | null> {
  const mine = await keys().where("uid", "==", uid).get();
  if (mine.docs.filter(d => !d.get("browser")).length >= MOST_API_KEYS) {
    return null;
  }
  const key = `ag_${kind === "secret" ? "sk" : "rk"}_live_${random62(32)}`;
  const id = hash(key);
  const now = Timestamp.now();
  const kept = {
    uid,
    kind,
    graphs: kind === "restricted" ? (graphs ?? []) : null,
    name: name.slice(0, 100),
    shown: shownKey(key),
    version: API_VERSION,
    createdAt: now,
    usedAt: null,
  };
  await keys().doc(id).create(kept);
  return {
    key,
    info: {
      id,
      kind,
      graphs: kept.graphs,
      version: API_VERSION,
      name: kept.name,
      shown: kept.shown,
      createdAt: now.toMillis(),
      usedAt: null,
    },
  };
}

/**
 * A new browser key for account `uid` (see above): read-only, reading every
 * graph it owns, for `BROWSER_KEY_MS`. Its others that have stopped
 * working are deleted, and the oldest if it has too many.
 */
export async function newBrowserKey(
  uid: string,
  now = Date.now(),
): Promise<{key: string; expires: number}> {
  const theirs = (await keys().where("uid", "==", uid).get()).docs
    .filter(d => d.get("browser"))
    .sort(
      (a, b) =>
        (a.get("createdAt") as Timestamp).toMillis() -
        (b.get("createdAt") as Timestamp).toMillis(),
    );
  const live = theirs.filter(
    d => (d.get("expireAt") as Timestamp).toMillis() > now,
  );
  const gone = [
    ...theirs.filter(d => !live.includes(d)),
    ...live.slice(0, Math.max(0, live.length - (MOST_BROWSER_KEYS - 1))),
  ];
  await Promise.all(gone.map(d => d.ref.delete()));

  const key = `ag_sk_live_${random62(32)}`;
  const expires = now + BROWSER_KEY_MS;
  await keys()
    .doc(hash(key))
    .create({
      uid,
      kind: "secret",
      graphs: null,
      name: "API showcase (a browser)",
      shown: shownKey(key),
      version: API_VERSION,
      createdAt: Timestamp.fromMillis(now),
      usedAt: null,
      browser: true,
      expireAt: Timestamp.fromMillis(expires),
    });
  return {key, expires};
}

/** The key `secret` is, if it's one that hasn't been revoked (or run out). */
export async function keyOf(secret: string): Promise<Key | null> {
  if (!KEY_PATTERN.test(secret)) {
    return null;
  }
  const ref = keys().doc(hash(secret));
  const snap = await ref.get();
  if (!snap.exists) {
    return null;
  }
  const data = snap.data() as {
    uid: string;
    kind: KeyKind;
    graphs: Array<string> | null;
    version?: string;
    usedAt?: Timestamp | null;
    expireAt?: Timestamp;
  };
  if (data.expireAt && data.expireAt.toMillis() <= Date.now()) {
    return null;
  }
  if (!data.usedAt || Date.now() - data.usedAt.toMillis() > USED_AT_EVERY_MS) {
    await ref.update({usedAt: Timestamp.now()}).catch(() => {});
  }
  return {
    id: snap.id,
    uid: data.uid,
    kind: data.kind,
    graphs: data.kind === "restricted" ? (data.graphs ?? []) : null,
    version: data.version ?? API_VERSION,
  };
}

/** Account `uid`'s keys, newest first. */
export async function keysOf(uid: string): Promise<Array<KeyInfo>> {
  const snap = await keys().where("uid", "==", uid).get();
  return snap.docs
    .filter(doc => !doc.get("browser"))
    .map(doc => {
      const d = doc.data();
      return {
        id: doc.id,
        kind: d.kind as KeyKind,
        graphs: (d.graphs as Array<string> | null) ?? null,
        version: (d.version as string) ?? API_VERSION,
        name: (d.name as string) ?? "",
        shown: (d.shown as string) ?? "",
        createdAt: (d.createdAt as Timestamp).toMillis(),
        usedAt: d.usedAt ? (d.usedAt as Timestamp).toMillis() : null,
      };
    })
    .sort((a, b) => b.createdAt - a.createdAt);
}

/** Revokes account `uid`'s key `id`; false if it has no such key. */
export async function revokeKey(uid: string, id: string): Promise<boolean> {
  if (!/^[0-9a-f]{64}$/.test(id)) {
    return false;
  }
  const ref = keys().doc(id);
  return firestore().runTransaction(async tx => {
    const snap = await tx.get(ref);
    if (!snap.exists || snap.get("uid") !== uid) {
      return false;
    }
    tx.delete(ref);
    return true;
  });
}

/** Deletes every key of account `uid` (the account's being deleted). */
export async function deleteKeysOf(uid: string): Promise<void> {
  const snap = await keys().where("uid", "==", uid).get();
  await Promise.all(snap.docs.map(d => d.ref.delete()));
}
