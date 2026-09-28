import { createHash, randomBytes } from "node:crypto";

import { Timestamp } from "firebase-admin/firestore";

import { safeEqual } from "./crypto";
import { openLogId, sealLogId } from "./encryption";
import { firestore } from "./firebase";

/**
 * What accounts keep, beyond Firebase Authentication's own records:
 *
 *   users/{uid}             { email, createdAt, lastLoginAt, lastWatchAt?, watch?: <sealed log id> }
 *   cli-tokens/{sha256}     { uid, email, host, createdAt, usedAt }
 *   cli-codes/{sha256}      { uid, email, challenge, expireAt }
 *
 * An account's document is made the first time it logs in to the site (it
 * signs up with Firebase, in the browser: `recordLogin`), and says when it
 * last logged in (in a browser, or the CLI on a computer) and last shared
 * live (`lastWatchAt`), and which share that was (`watch`: /watch shows it).
 *
 * `agent-graph watch-remote` logs in through the browser: it opens
 * /login?cli=<port>&state=…&challenge=…, and once the user is logged in the
 * page asks for a one-time code (`newCliCode`) and sends the browser to
 * http://127.0.0.1:<port>/callback with it. The CLI trades the code, with
 * the secret its challenge was made from (PKCE, RFC 7636), for a token of
 * its own (`redeemCliCode`), which it keeps and sends to make shares. Codes
 * and tokens are stored by their SHA-256, never as they are: the database
 * alone can't be used to act as anyone. A token lasts until the user logs
 * out on that computer (`agent-graph watch-remote --logout`) or removes it
 * on their account page.
 */

/** How long a code is good for: the CLI trades it at once. */
export const CLI_CODE_MS = 5 * 60 * 1000;

/** How often, at most, a token's `usedAt` is brought up to date. */
const USED_AT_EVERY_MS = 24 * 60 * 60 * 1000;

const users = () => firestore().collection("users");
const tokens = () => firestore().collection("cli-tokens");
const codes = () => firestore().collection("cli-codes");

const hash = (value: string) => createHash("sha256").update(value).digest("hex");

/** A PKCE challenge, or `state`: 43 characters of base64url (32 bytes). */
export const SECRET_PATTERN = /^[A-Za-z0-9_-]{43}$/;

export type Account = { uid: string; email: string | null };

/** A one-time code for `account`, for the CLI whose challenge is `challenge`. */
export async function newCliCode(account: Account, challenge: string): Promise<string> {
  const code = randomBytes(32).toString("base64url");
  await codes()
    .doc(hash(code))
    .create({
      uid: account.uid,
      email: account.email,
      challenge,
      // (Firestore can delete expired ones itself, with a TTL policy on expireAt.)
      expireAt: Timestamp.fromMillis(Date.now() + CLI_CODE_MS),
    });
  return code;
}

/**
 * The account a code was for, if it's `verifier`'s (its SHA-256, base64url,
 * is the challenge) and hasn't expired. A code is used once: it's deleted,
 * whether or not the verifier matches.
 */
export async function redeemCliCode(code: string, verifier: string): Promise<Account | null> {
  const ref = codes().doc(hash(code));
  const data = await firestore().runTransaction(async (tx) => {
    const snap = await tx.get(ref);
    if (!snap.exists) return null;
    tx.delete(ref);
    return snap.data() as { uid: string; email: string | null; challenge: string; expireAt: Timestamp };
  });
  if (!data || data.expireAt.toMillis() < Date.now()) return null;
  const challenge = createHash("sha256").update(verifier).digest("base64url");
  return safeEqual(challenge, data.challenge) ? { uid: data.uid, email: data.email } : null;
}

/** A new token for the CLI on computer `host`, logged in as `account`. */
export async function newCliToken(account: Account, host: string): Promise<string> {
  const token = `agt_${randomBytes(32).toString("base64url")}`;
  const now = Timestamp.now();
  await tokens()
    .doc(hash(token))
    .create({
      uid: account.uid,
      email: account.email,
      host: host.slice(0, 100),
      createdAt: now,
      usedAt: now,
    });
  return token;
}

/** The account a CLI token is for, if it's one. */
export async function accountOfToken(token: string): Promise<Account | null> {
  if (!/^agt_[A-Za-z0-9_-]{43}$/.test(token)) return null;
  const ref = tokens().doc(hash(token));
  const snap = await ref.get();
  if (!snap.exists) return null;
  const { uid, email, usedAt } = snap.data() as { uid: string; email: string | null; usedAt?: Timestamp };
  if (!usedAt || Date.now() - usedAt.toMillis() > USED_AT_EVERY_MS) {
    await ref.update({ usedAt: Timestamp.now() }).catch(() => {});
  }
  return { uid, email };
}

/** The account a request's `Authorization: Bearer <CLI token>` is for, if any. */
export async function accountOfRequest(req: Request): Promise<Account | null> {
  const header = req.headers.get("authorization") ?? "";
  return header.startsWith("Bearer ") ? accountOfToken(header.slice(7).trim()) : null;
}

/** Forgets a CLI token: that computer is logged out. */
export async function deleteCliToken(token: string): Promise<void> {
  await tokens().doc(hash(token)).delete();
}

export type Computer = { id: string; host: string; createdAt: number; usedAt: number };

/** The computers `uid` is logged in on with the CLI, most recently used first. */
export async function computersOf(uid: string): Promise<Computer[]> {
  const snap = await tokens().where("uid", "==", uid).get();
  return snap.docs
    .map((doc) => {
      const d = doc.data();
      return {
        id: doc.id,
        host: String(d.host ?? ""),
        createdAt: (d.createdAt as Timestamp).toMillis(),
        usedAt: ((d.usedAt ?? d.createdAt) as Timestamp).toMillis(),
      };
    })
    .sort((a, b) => b.usedAt - a.usedAt);
}

/** Logs `uid` out of the CLI on one computer (`id`, from `computersOf`). */
export async function removeComputer(uid: string, id: string): Promise<boolean> {
  if (!/^[0-9a-f]{64}$/.test(id)) return false;
  const ref = tokens().doc(id);
  const snap = await ref.get();
  if (!snap.exists || snap.get("uid") !== uid) return false;
  await ref.delete();
  return true;
}

/**
 * Records that `account` logged in, just now (in a browser, or the CLI on a
 * computer): its document is made the first time, with its email; later,
 * its email (which can change) and when it last logged in are brought up
 * to date.
 */
export async function recordLogin(account: Account): Promise<void> {
  const ref = users().doc(account.uid);
  await firestore().runTransaction(async (tx) => {
    const now = Timestamp.now();
    const snap = await tx.get(ref);
    if (snap.exists) tx.update(ref, { email: account.email, lastLoginAt: now });
    else tx.set(ref, { email: account.email, createdAt: now, lastLoginAt: now });
  });
}

/** Makes log `id` the one /watch shows `uid`: they're sharing live, now. */
export async function setWatchLog(uid: string, id: string): Promise<void> {
  await users()
    .doc(uid)
    .set({ watch: sealLogId(uid, id), lastWatchAt: Timestamp.now() }, { merge: true });
}

/** The log /watch shows `uid`: their latest live share, if they have one. */
export async function watchLog(uid: string): Promise<string | null> {
  const snap = await users().doc(uid).get();
  const sealed = snap.get("watch") as Uint8Array | undefined;
  return sealed ? openLogId(uid, sealed) : null;
}
