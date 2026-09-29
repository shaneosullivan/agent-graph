// Accounts, end to end, against a running copy of the site and the Auth and
// Firestore emulators (lib/auth.ts, lib/accounts.ts).
//
//   npm run emulators            # in one terminal
//   FIREBASE_AUTH_EMULATOR_HOST=127.0.0.1:9099 npm run dev   # in another
//   BASE_URL=http://localhost:3000 npm run test:api
//
// Paying for live shares (lib/billing.ts) is tested when STRIPE_MODE and
// its settings are set, for the site and the tests alike
// (scripts/ci-api-test.sh sets them); nothing here talks to Stripe itself.

import assert from "node:assert/strict";
import {createHash, randomBytes} from "node:crypto";
import {test} from "node:test";

const BASE = (process.env.BASE_URL || "http://localhost:3000").replace(
  /\/$/,
  "",
);
const AUTH = process.env.FIREBASE_AUTH_EMULATOR_HOST || "127.0.0.1:9099";
const FIRESTORE = process.env.FIRESTORE_EMULATOR_HOST || "127.0.0.1:8080";
const PROJECT =
  process.env.GCLOUD_PROJECT ||
  process.env.FIREBASE_PROJECT_ID ||
  "demo-agent-graph";
const FROM_SITE = {Origin: BASE};

const line = n =>
  JSON.stringify({
    v: 1,
    id: `01K0000000000000000000${String(n).padStart(4, "0")}`,
    ts: "2026-09-25T10:00:00.000Z",
    type: "status",
    node: "x:s",
    data: {state: "working"},
  }) + "\n";

const secret = () => randomBytes(32).toString("base64url");

/** A new account in the emulator, and a fresh sign-in's ID token (as the browser gets). */
async function signUp() {
  const email = `t-${randomBytes(6).toString("hex")}@agent-graph.test`;
  const password = secret();
  const res = await fetch(
    `http://${AUTH}/identitytoolkit.googleapis.com/v1/accounts:signUp?key=demo-key`,
    {
      method: "POST",
      headers: {"Content-Type": "application/json"},
      body: JSON.stringify({email, password, returnSecureToken: true}),
    },
  );
  assert.equal(res.status, 200, await res.clone().text());
  const {idToken, localId} = await res.json();
  return {email, password, uid: localId, idToken};
}

/** Signing in again, as the browser would: a fresh ID token. */
async function signIn(email, password) {
  const res = await fetch(
    `http://${AUTH}/identitytoolkit.googleapis.com/v1/accounts:signInWithPassword?key=demo-key`,
    {
      method: "POST",
      headers: {"Content-Type": "application/json"},
      body: JSON.stringify({email, password, returnSecureToken: true}),
    },
  );
  assert.equal(res.status, 200, await res.clone().text());
  return (await res.json()).idToken;
}

/** Account `uid`'s document in Firestore (the emulator), its fields as plain values; null if there's none. */
async function userDoc(uid) {
  const res = await fetch(
    `http://${FIRESTORE}/v1/projects/${PROJECT}/databases/(default)/documents/users/${uid}`,
    {headers: {Authorization: "Bearer owner"}},
  );
  if (res.status === 404) return null;
  assert.equal(res.status, 200, await res.clone().text());
  const {fields} = await res.json();
  return Object.fromEntries(
    Object.entries(fields).map(([k, v]) => [
      k,
      "timestampValue" in v
        ? Date.parse(v.timestampValue)
        : (v.stringValue ?? v.bytesValue ?? v.nullValue),
    ]),
  );
}

/** Logs a browser in with an ID token: its cookies. */
async function session(idToken) {
  const res = await fetch(`${BASE}/api/session`, {
    method: "POST",
    headers: {"Content-Type": "application/json", ...FROM_SITE},
    body: JSON.stringify({idToken}),
  });
  assert.equal(res.status, 204, await res.clone().text());
  return res.headers
    .getSetCookie()
    .map(c => c.split(";")[0])
    .join("; ");
}

/** A browser logged in as a new account: its cookies, for a Cookie header. */
async function loggedIn() {
  const {email, uid, idToken} = await signUp();
  const res = await fetch(`${BASE}/api/session`, {
    method: "POST",
    headers: {"Content-Type": "application/json", ...FROM_SITE},
    body: JSON.stringify({idToken}),
  });
  assert.equal(res.status, 204, await res.clone().text());
  const cookie = res.headers
    .getSetCookie()
    .map(c => c.split(";")[0])
    .join("; ");
  return {email, uid, cookie};
}

/** Sets fields of account `uid`'s document in Firestore (the emulator). */
async function setUser(uid, fields) {
  const value = v =>
    v instanceof Date
      ? {timestampValue: v.toISOString()}
      : typeof v === "string"
        ? {stringValue: v}
        : {
            mapValue: {
              fields: Object.fromEntries(
                Object.entries(v).map(([k, x]) => [k, value(x)]),
              ),
            },
          };
  const mask = Object.keys(fields)
    .map(k => `updateMask.fieldPaths=${k}`)
    .join("&");
  const res = await fetch(
    `http://${FIRESTORE}/v1/projects/${PROJECT}/databases/(default)/documents/users/${uid}?${mask}`,
    {
      method: "PATCH",
      headers: {
        Authorization: "Bearer owner",
        "Content-Type": "application/json",
      },
      body: JSON.stringify({
        fields: Object.fromEntries(
          Object.entries(fields).map(([k, v]) => [k, value(v)]),
        ),
      }),
    },
  );
  assert.equal(res.status, 200, await res.clone().text());
}

/** What the login page does for the CLI: a one-time code for its challenge. */
async function cliCode(cookie, verifier) {
  const challenge = createHash("sha256").update(verifier).digest("base64url");
  const state = secret();
  const res = await fetch(`${BASE}/api/cli/code`, {
    method: "POST",
    headers: {"Content-Type": "application/json", Cookie: cookie, ...FROM_SITE},
    body: JSON.stringify({port: 43210, state, challenge}),
  });
  assert.equal(res.status, 200, await res.clone().text());
  const redirect = new URL((await res.json()).redirect);
  assert.equal(redirect.origin, "http://127.0.0.1:43210");
  assert.equal(redirect.pathname, "/callback");
  assert.equal(redirect.searchParams.get("state"), state);
  return redirect.searchParams.get("code");
}

function trade(code, verifier) {
  return fetch(`${BASE}/api/cli/token`, {
    method: "POST",
    headers: {"Content-Type": "application/json"},
    body: JSON.stringify({code, verifier, host: "test-computer"}),
  });
}

/** An account logged in in a browser, and on a computer (the CLI's token). */
async function withCli() {
  const browser = await loggedIn();
  const verifier = secret();
  const res = await trade(await cliCode(browser.cookie, verifier), verifier);
  assert.equal(res.status, 200, await res.clone().text());
  const {token, email} = await res.json();
  assert.equal(email, browser.email);
  return {...browser, token};
}

function share(token, body = line(1)) {
  return fetch(`${BASE}/api/logs`, {
    method: "POST",
    headers: {
      "Content-Type": "application/x-ndjson",
      "X-Agent-Graph-Source": "watch",
      ...(token ? {Authorization: `Bearer ${token}`} : {}),
    },
    body,
  });
}

const content = (id, cookie) =>
  fetch(`${BASE}/api/logs/${id}/content`, {
    headers: cookie ? {Cookie: cookie} : {},
  });

test("a browser logs in only from the site's own pages", async () => {
  const {email, idToken} = await signUp();
  const body = JSON.stringify({idToken});
  const headers = {"Content-Type": "application/json"};
  for (const origin of [undefined, "https://evil.example"]) {
    const res = await fetch(`${BASE}/api/session`, {
      method: "POST",
      headers: origin ? {...headers, Origin: origin} : headers,
      body,
    });
    assert.equal(res.status, 403, `from ${origin}`);
  }
  const res = await fetch(`${BASE}/api/session`, {
    method: "POST",
    headers: {...headers, ...FROM_SITE},
    body,
  });
  assert.equal(res.status, 204);
  const session = res.headers
    .getSetCookie()
    .find(c => c.startsWith("__session="));
  assert.match(session, /HttpOnly/);
  assert.match(session, /SameSite=Lax/);
  const cookie = res.headers
    .getSetCookie()
    .map(c => c.split(";")[0])
    .join("; ");
  const page = await fetch(`${BASE}/account`, {headers: {Cookie: cookie}});
  assert.ok((await page.text()).includes(email), "logged in as them");
  // Not a token: the session cookie only comes from a real sign-in.
  const bad = await fetch(`${BASE}/api/session`, {
    method: "POST",
    headers: {...headers, ...FROM_SITE},
    body: JSON.stringify({idToken: "not-a-token"}),
  });
  assert.equal(bad.status, 401);
});

test("the CLI's code is used once, and only with its secret", async () => {
  const {cookie} = await loggedIn();
  const verifier = secret();
  const code = await cliCode(cookie, verifier);
  assert.equal((await trade(code, secret())).status, 401, "another secret");
  assert.equal(
    (await trade(code, verifier)).status,
    401,
    "used up, even by the wrong secret",
  );

  const again = await cliCode(cookie, verifier);
  assert.equal((await trade(again, verifier)).status, 200);
  assert.equal((await trade(again, verifier)).status, 401, "once");

  // Asking for a code needs a browser logged in, on the site's own pages.
  const res = await fetch(`${BASE}/api/cli/code`, {
    method: "POST",
    headers: {"Content-Type": "application/json", ...FROM_SITE},
    body: JSON.stringify({port: 43210, state: secret(), challenge: secret()}),
  });
  assert.equal(res.status, 401);
});

test("a live share needs a login, and is only its owner's", async () => {
  assert.equal((await share(null)).status, 401, "not logged in");
  assert.equal((await share("agt_" + secret())).status, 401, "not a login");

  const owner = await withCli();
  const res = await share(owner.token);
  assert.equal(res.status, 201, await res.clone().text());
  const {id, url} = await res.json();
  assert.equal(new URL(url).pathname, "/watch");

  const other = await loggedIn();
  assert.equal((await content(id)).status, 401, "logged out");
  assert.equal(
    (await content(id, other.cookie)).status,
    401,
    "another account",
  );
  const mine = await content(id, owner.cookie);
  assert.equal(mine.status, 200);
  assert.equal(await mine.text(), line(1));

  // Its own address is only there for its owner.
  const page = cookie =>
    fetch(`${BASE}/l/${id}`, {
      headers: cookie ? {Cookie: cookie} : {},
      redirect: "manual",
    });
  assert.equal((await page()).status, 404);
  assert.equal((await page(other.cookie)).status, 404);
  assert.equal((await page(owner.cookie)).status, 200);

  // /watch: the owner's latest share; to others, their own (none).
  const watch = cookie =>
    fetch(`${BASE}/watch`, {
      headers: cookie ? {Cookie: cookie} : {},
      redirect: "manual",
    });
  const out = await watch();
  assert.equal(out.status, 307);
  assert.equal(
    out.headers.get("location").replace(BASE, ""),
    "/login?next=/watch",
  );
  const theirs = await (await watch(owner.cookie)).text();
  assert.ok(theirs.includes(`"id":"${id}"`), "the owner's share");
  const others = await (await watch(other.cookie)).text();
  assert.ok(!others.includes(id) && others.includes("Nothing shared yet"));
});

test("carrying on with a share needs its owner's login and its key", async () => {
  const owner = await withCli();
  const {id, writeToken} = await (await share(owner.token)).json();
  const second = await (await share(owner.token, line(2))).json();

  const claim = (token, key) =>
    fetch(`${BASE}/api/logs/${id}/watch`, {
      method: "POST",
      headers: {
        Authorization: `Bearer ${token}`,
        "X-Agent-Graph-Write-Token": key,
      },
    });
  const someone = await withCli();
  assert.equal(
    (await claim(someone.token, writeToken)).status,
    404,
    "another account's",
  );
  assert.equal(
    (await claim(owner.token, second.writeToken)).status,
    401,
    "another share's key",
  );
  assert.equal(
    (await claim("agt_" + secret(), writeToken)).status,
    401,
    "not a login",
  );

  // The latest is the second; carrying on with the first makes it the latest.
  const latest = async () =>
    await (
      await fetch(`${BASE}/watch`, {headers: {Cookie: owner.cookie}})
    ).text();
  assert.ok((await latest()).includes(`"id":"${second.id}"`));
  const res = await claim(owner.token, writeToken);
  assert.equal(res.status, 200);
  assert.equal(new URL((await res.json()).url).pathname, "/watch");
  assert.ok((await latest()).includes(`"id":"${id}"`));
});

test("logging the CLI out ends its login", async () => {
  const {token} = await withCli();
  assert.equal((await share(token)).status, 201);
  const out = await fetch(`${BASE}/api/cli/logout`, {
    method: "POST",
    headers: {Authorization: `Bearer ${token}`},
  });
  assert.equal(out.status, 204);
  assert.equal((await share(token)).status, 401);
});

test("an account's document: made at its first login, then kept up to date", async () => {
  const {email, password, uid, idToken} = await signUp();
  assert.equal(
    await userDoc(uid),
    null,
    "signing up alone (with Firebase) doesn't make it",
  );

  const cookie = await session(idToken);
  const first = await userDoc(uid);
  assert.equal(first.email, email);
  assert.ok(first.createdAt > 0);
  assert.equal(first.lastLoginAt, first.createdAt);
  assert.equal(first.lastWatchAt, undefined, "hasn't shared yet");

  await new Promise(r => setTimeout(r, 20));
  await session(await signIn(email, password));
  const again = await userDoc(uid);
  assert.equal(again.createdAt, first.createdAt, "made once");
  assert.ok(again.lastLoginAt > first.lastLoginAt, "logged in again");

  // The CLI logging in counts, and sharing is when it last watched.
  await new Promise(r => setTimeout(r, 20));
  const verifier = secret();
  const res = await trade(await cliCode(cookie, verifier), verifier);
  const {token} = await res.json();
  const cli = await userDoc(uid);
  assert.ok(cli.lastLoginAt > again.lastLoginAt, "the CLI logged in");
  assert.equal((await share(token)).status, 201);
  const shared = await userDoc(uid);
  assert.ok(shared.lastWatchAt >= cli.lastLoginAt, "shared live");
  assert.ok(shared.watch, "and which share");
});

// ---------- paying for live shares (lib/billing.ts) ----------

// (Set with every setting its mode needs: scripts/ci-api-test.sh.)
const BILLING = Boolean(process.env.STRIPE_MODE);
const FREE_DAYS = Number(process.env.FREE_TRIAL_DAYS || 7);
const DAY = 24 * 60 * 60 * 1000;
const paid = {
  status: "active",
  subscription: {periodEnd: new Date(Date.now() + 30 * DAY)},
};

const claim = (id, token, key) =>
  fetch(`${BASE}/api/logs/${id}/watch`, {
    method: "POST",
    headers: {
      Authorization: `Bearer ${token}`,
      "X-Agent-Graph-Write-Token": key,
    },
  });

const append = (id, key, offset, body) =>
  fetch(`${BASE}/api/logs/${id}/append?offset=${offset}`, {
    method: "POST",
    headers: {
      "Content-Type": "application/x-ndjson",
      Authorization: `Bearer ${key}`,
    },
    body,
  });

const standing = async token =>
  (
    await fetch(`${BASE}/api/cli/account`, {
      headers: {Authorization: `Bearer ${token}`},
    })
  ).json();

test("an account's made unpaid, and told where it stands with a share", async () => {
  const {uid, token, email} = await withCli();
  assert.equal((await userDoc(uid)).status, "unpaid");
  const res = await share(token);
  assert.equal(res.status, 201);
  const reply = await res.json();
  const now = await standing(token);
  assert.equal(now.email, email);
  assert.equal(now.canShare, true);
  if (BILLING) {
    const made = (await userDoc(uid)).createdAt;
    assert.equal(reply.accountStatus, "unpaid");
    assert.equal(reply.freeUntil, made + FREE_DAYS * DAY);
    assert.equal(now.accountStatus, "unpaid");
  } else {
    assert.equal(reply.accountStatus, "active", "the site's free");
    assert.equal(reply.freeUntil, null);
  }
  const out = await fetch(`${BASE}/api/cli/account`, {
    headers: {Authorization: `Bearer agt_${secret()}`},
  });
  assert.equal(out.status, 401, "not a login");
});

test(
  "once its free days are over, an account subscribes to share live",
  {skip: !BILLING && "Stripe's settings aren't set"},
  async () => {
    const {uid, token} = await withCli();
    const {id, writeToken} = await (await share(token)).json();

    await setUser(uid, {
      createdAt: new Date(Date.now() - (FREE_DAYS + 1) * DAY),
    });
    const refused = await share(token);
    assert.equal(refused.status, 402);
    assert.match(await refused.text(), /\/account/, "says where to subscribe");
    assert.equal((await claim(id, token, writeToken)).status, 402);
    const now = await standing(token);
    assert.equal(now.accountStatus, "unpaid");
    assert.equal(now.canShare, false);

    await setUser(uid, paid);
    const res = await share(token);
    assert.equal(res.status, 201);
    const reply = await res.json();
    assert.equal(reply.accountStatus, "active");
    assert.equal(reply.freeUntil, null);
    assert.equal((await claim(id, token, writeToken)).status, 200);
    assert.equal((await standing(token)).canShare, true);
  },
);

test(
  "a share's appends stop when its account's time is up, till it's started again",
  {skip: !BILLING && "Stripe's settings aren't set"},
  async () => {
    const {uid, token} = await withCli();
    // Its free days end in a moment.
    await setUser(uid, {
      createdAt: new Date(Date.now() - FREE_DAYS * DAY + 3000),
    });
    const {id, writeToken} = await (await share(token)).json();
    let offset = Buffer.byteLength(line(1));
    const next = async n => {
      const res = await append(id, writeToken, offset, line(n));
      if (res.status === 204) {
        offset += Buffer.byteLength(line(n));
      }
      return res.status;
    };
    assert.equal(await next(2), 204, "still free");
    await new Promise(r => setTimeout(r, 3500));
    assert.equal(await next(3), 402, "time's up");
    assert.equal((await claim(id, token, writeToken)).status, 402, "not paid");

    await setUser(uid, paid);
    assert.equal(await next(3), 402, "not till it's started again");
    assert.equal((await claim(id, token, writeToken)).status, 200);
    assert.equal(await next(3), 204, "paid: it carries on");
  },
);

test("Stripe's webhook takes only what Stripe signed", async () => {
  const hook = headers =>
    fetch(`${BASE}/api/stripe/webhook`, {
      method: "POST",
      headers: {"Content-Type": "application/json", ...headers},
      body: JSON.stringify({type: "customer.subscription.updated"}),
    });
  if (!BILLING) {
    assert.equal((await hook({})).status, 404, "Stripe isn't set up");
    return;
  }
  assert.equal((await hook({})).status, 400, "unsigned");
  assert.equal(
    (
      await hook({
        "Stripe-Signature": `t=${Math.floor(Date.now() / 1000)},v1=00`,
      })
    ).status,
    400,
    "signed by someone else",
  );
});

test(
  "subscribing is to a plan, monthly or yearly",
  {skip: !BILLING && "Stripe's settings aren't set"},
  async () => {
    const {cookie} = await loggedIn();
    for (const body of ["", "{}", JSON.stringify({plan: "weekly"})]) {
      const res = await fetch(`${BASE}/api/account/subscribe`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          Cookie: cookie,
          ...FROM_SITE,
        },
        body,
      });
      assert.equal(res.status, 400, body);
    }
  },
);

test("subscribing needs a browser logged in, on the site's own pages", async () => {
  for (const path of ["/api/account/subscribe", "/api/account/billing"]) {
    const res = await fetch(`${BASE}${path}`, {method: "POST"});
    assert.equal(res.status, 403, `${path}, from elsewhere`);
    const out = await fetch(`${BASE}${path}`, {
      method: "POST",
      headers: FROM_SITE,
    });
    assert.equal(out.status, 401, `${path}, logged out`);
  }
});
