import {createHash, randomBytes} from "node:crypto";

import {Timestamp} from "firebase-admin/firestore";

import {
  billingConfig,
  type Paying,
  type Plan,
  type Standing,
  standing,
} from "./billing";
import {safeEqual} from "./crypto";
import {openLogId, sealLogId} from "./encryption";
import {firestore} from "./firebase";

/**
 * What accounts keep, beyond Firebase Authentication's own records:
 *
 *   users/{uid}             { email, createdAt, lastLoginAt, lastWatchAt?, watch?: <sealed log id>,
 *                             status, stripeCustomer?, subscription?: { id, status, plan, periodEnd, cancelAt } }
 *   cli-tokens/{sha256}     { uid, email, host, createdAt, usedAt }
 *   cli-codes/{sha256}      { uid, email, challenge, expireAt }
 *
 * An account's document is made the first time it logs in to the site (it
 * signs up with Firebase, in the browser: `recordLogin`), and says when it
 * last logged in (in a browser, or the CLI on a computer) and last shared
 * live (`lastWatchAt`), and which share that was (`watch`: /watch shows it).
 * It's made `unpaid`; once it subscribes, it's `active` for as long as it's
 * paid for (`subscription`, from Stripe: lib/stripe.ts). Where it stands,
 * and whether it can share live, is lib/billing.ts's to say.
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

const hash = (value: string) =>
  createHash("sha256").update(value).digest("hex");

/** A PKCE challenge, or `state`: 43 characters of base64url (32 bytes). */
export const SECRET_PATTERN = /^[A-Za-z0-9_-]{43}$/;

export type Account = {uid: string; email: string | null};

/** Where account `uid` stands now: whether it can share live, and until when. */
export async function standingOf(uid: string): Promise<Standing> {
  const billing = billingConfig();
  if (!billing) {
    return standing(null, null, Date.now());
  }
  return standing(billing, await payingOf(uid), Date.now());
}

/** What account `uid`'s document says about paying; null if it has none. */
export async function payingOf(uid: string): Promise<Paying | null> {
  const snap = await users().doc(uid).get();
  if (!snap.exists) {
    return null;
  }
  const d = snap.data() as {
    status?: Paying["status"];
    createdAt?: Timestamp;
    subscription?: {periodEnd?: Timestamp};
  };
  return {
    status: d.status,
    createdAt: d.createdAt?.toMillis(),
    periodEnd: d.subscription?.periodEnd?.toMillis(),
  };
}

/** Account `uid`'s Stripe customer, if it has one yet. */
export async function stripeCustomerOf(uid: string): Promise<string | null> {
  const snap = await users().doc(uid).get();
  return (snap.get("stripeCustomer") as string | undefined) ?? null;
}

export async function setStripeCustomer(
  uid: string,
  customer: string,
): Promise<void> {
  await users().doc(uid).set({stripeCustomer: customer}, {merge: true});
}

export type Subscription = {
  id: string;
  /** Stripe's own status for it: active, trialing, past_due, canceled, … */
  status: string;
  /** Which plan it's on; null if its price isn't one of STRIPE_MODE's now. */
  plan: Plan | null;
  /** When the period paid for (or its trial) ends, in ms. */
  periodEnd: number | null;
  /** When it's set to end (canceled, at the period's end), in ms. */
  cancelAt: number | null;
};

/**
 * Records account `uid`'s subscription, as Stripe has it: it's `active`
 * while Stripe has it active (or in a trial), and `unpaid` otherwise.
 */
export async function setSubscription(
  uid: string,
  customer: string,
  sub: Subscription,
): Promise<void> {
  const paid = sub.status === "active" || sub.status === "trialing";
  const ts = (ms: number | null) =>
    ms === null ? null : Timestamp.fromMillis(ms);
  await users()
    .doc(uid)
    .set(
      {
        status: paid ? "active" : "unpaid",
        stripeCustomer: customer,
        subscription: {
          id: sub.id,
          status: sub.status,
          plan: sub.plan,
          periodEnd: ts(sub.periodEnd),
          cancelAt: ts(sub.cancelAt),
        },
      },
      {merge: true},
    );
}

/** Account `uid`'s subscription, as last recorded, if it's had one. */
export async function subscriptionOf(
  uid: string,
): Promise<Subscription | null> {
  const snap = await users().doc(uid).get();
  const sub = snap.get("subscription") as
    | {
        id: string;
        status: string;
        plan?: Plan | null;
        periodEnd: Timestamp | null;
        cancelAt: Timestamp | null;
      }
    | undefined;
  return sub
    ? {
        id: sub.id,
        status: sub.status,
        plan: sub.plan ?? null,
        periodEnd: sub.periodEnd?.toMillis() ?? null,
        cancelAt: sub.cancelAt?.toMillis() ?? null,
      }
    : null;
}

/** A one-time code for `account`, for the CLI whose challenge is `challenge`. */
export async function newCliCode(
  account: Account,
  challenge: string,
): Promise<string> {
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
export async function redeemCliCode(
  code: string,
  verifier: string,
): Promise<Account | null> {
  const ref = codes().doc(hash(code));
  const data = await firestore().runTransaction(async tx => {
    const snap = await tx.get(ref);
    if (!snap.exists) {
      return null;
    }
    tx.delete(ref);
    return snap.data() as {
      uid: string;
      email: string | null;
      challenge: string;
      expireAt: Timestamp;
    };
  });
  if (!data || data.expireAt.toMillis() < Date.now()) {
    return null;
  }
  const challenge = createHash("sha256").update(verifier).digest("base64url");
  return safeEqual(challenge, data.challenge)
    ? {uid: data.uid, email: data.email}
    : null;
}

/** A new token for the CLI on computer `host`, logged in as `account`. */
export async function newCliToken(
  account: Account,
  host: string,
): Promise<string> {
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
  if (!/^agt_[A-Za-z0-9_-]{43}$/.test(token)) {
    return null;
  }
  const ref = tokens().doc(hash(token));
  const snap = await ref.get();
  if (!snap.exists) {
    return null;
  }
  const {uid, email, usedAt} = snap.data() as {
    uid: string;
    email: string | null;
    usedAt?: Timestamp;
  };
  if (!usedAt || Date.now() - usedAt.toMillis() > USED_AT_EVERY_MS) {
    await ref.update({usedAt: Timestamp.now()}).catch(() => {});
  }
  return {uid, email};
}

/** The account a request's `Authorization: Bearer <CLI token>` is for, if any. */
export async function accountOfRequest(req: Request): Promise<Account | null> {
  const header = req.headers.get("authorization") ?? "";
  return header.startsWith("Bearer ")
    ? accountOfToken(header.slice(7).trim())
    : null;
}

/** Forgets a CLI token: that computer is logged out. */
export async function deleteCliToken(token: string): Promise<void> {
  await tokens().doc(hash(token)).delete();
}

export type Computer = {
  id: string;
  host: string;
  createdAt: number;
  usedAt: number;
};

/** The computers `uid` is logged in on with the CLI, most recently used first. */
export async function computersOf(uid: string): Promise<Array<Computer>> {
  const snap = await tokens().where("uid", "==", uid).get();
  return snap.docs
    .map(doc => {
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
export async function removeComputer(
  uid: string,
  id: string,
): Promise<boolean> {
  if (!/^[0-9a-f]{64}$/.test(id)) {
    return false;
  }
  const ref = tokens().doc(id);
  const snap = await ref.get();
  if (!snap.exists || snap.get("uid") !== uid) {
    return false;
  }
  await ref.delete();
  return true;
}

/**
 * Records that `account` logged in, just now (in a browser, or the CLI on a
 * computer): its document is made the first time, with its email, and
 * `unpaid` (lib/billing.ts); later, its email (which can change) and when it
 * last logged in are brought up to date.
 */
export async function recordLogin(account: Account): Promise<void> {
  const ref = users().doc(account.uid);
  await firestore().runTransaction(async tx => {
    const now = Timestamp.now();
    const snap = await tx.get(ref);
    if (snap.exists) {
      tx.update(ref, {email: account.email, lastLoginAt: now});
    } else {
      tx.set(ref, {
        email: account.email,
        createdAt: now,
        lastLoginAt: now,
        status: "unpaid",
      });
    }
  });
}

/** Makes log `id` the one /watch shows `uid`: they're sharing live, now. */
export async function setWatchLog(uid: string, id: string): Promise<void> {
  await users()
    .doc(uid)
    .set(
      {watch: sealLogId(uid, id), lastWatchAt: Timestamp.now()},
      {merge: true},
    );
}

/** The log /watch shows `uid`: their latest live share, if they have one. */
export async function watchLog(uid: string): Promise<string | null> {
  const snap = await users().doc(uid).get();
  const sealed = snap.get("watch") as Uint8Array | undefined;
  return sealed ? openLogId(uid, sealed) : null;
}
