// The site's API, end to end, against a running copy of the site.
//
//   npm run emulators            # in one terminal
//   npm run dev                  # in another
//   BASE_URL=http://localhost:3000 npm run test:api

import assert from "node:assert/strict";
import { test } from "node:test";

const BASE = (process.env.BASE_URL || "http://localhost:3000").replace(/\/$/, "");

const line = (n, state = "working") =>
  JSON.stringify({
    v: 1,
    id: `01K0000000000000000000${String(n).padStart(4, "0")}`,
    ts: "2026-09-25T10:00:00.000Z",
    type: "status",
    node: "x:s",
    data: { state },
  }) + "\n";

const b64url = (s) => Buffer.from(s, "utf8").toString("base64url");

async function create(body, headers = {}) {
  const res = await fetch(`${BASE}/api/logs`, {
    method: "POST",
    headers: { "Content-Type": "application/x-ndjson", ...headers },
    body,
  });
  assert.equal(res.status, 201, await res.clone().text());
  return res.json();
}

function append(id, offset, body, key) {
  return fetch(`${BASE}/api/logs/${id}/append?offset=${offset}`, {
    method: "POST",
    headers: { "Content-Type": "application/x-ndjson", ...(key ? { Authorization: `Bearer ${key}` } : {}) },
    body,
  });
}

async function content(id, after = "", cookie) {
  const res = await fetch(`${BASE}/api/logs/${id}/content?after=${after}`, {
    headers: cookie ? { Cookie: cookie } : {},
  });
  return { status: res.status, text: await res.text(), last: res.headers.get("x-last-chunk") };
}

test("create, append with the key, and read back in order", async () => {
  const first = line(1) + line(2);
  const log = await create(first, { "X-Agent-Graph-Source": "watch" });
  assert.match(log.id, /^[A-Za-z0-9]{12}$/);
  assert.equal(log.url, `${BASE}/l/${log.id}`);

  assert.equal((await append(log.id, first.length, line(3), log.writeToken)).status, 204);
  const all = await content(log.id);
  assert.equal(all.status, 200);
  assert.equal(all.text, first + line(3));

  // Reading on from the last chunk gets only what's new.
  assert.equal((await content(log.id, all.last)).text, "");
  assert.equal((await append(log.id, (first + line(3)).length, line(4), log.writeToken)).status, 204);
  assert.equal((await content(log.id, all.last)).text, line(4));
});

test("updates need the key the log was created with", async () => {
  const mine = await create(line(1));
  const theirs = await create(line(1));
  assert.equal((await append(mine.id, 100, line(9))).status, 401, "no key");
  assert.equal((await append(mine.id, 100, line(9), "guess")).status, 401, "wrong key");
  assert.equal((await append(mine.id, 100, line(9), theirs.writeToken)).status, 401, "another log's key");
  assert.equal((await content(mine.id)).text, line(1), "nothing was added");
});

test("retrying a chunk doesn't duplicate it", async () => {
  const log = await create(line(1));
  const offset = line(1).length;
  await append(log.id, offset, line(2), log.writeToken);
  await append(log.id, offset, line(2), log.writeToken);
  assert.equal((await content(log.id)).text, line(1) + line(2));
});

test("password-protected logs need the password to read", async () => {
  const log = await create(line(1), { "X-Agent-Graph-Password": b64url("pässwörd") });
  assert.equal((await content(log.id)).status, 401);

  const wrong = await fetch(`${BASE}/api/logs/${log.id}/unlock`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ password: "nope" }),
  });
  assert.equal(wrong.status, 401);

  const right = await fetch(`${BASE}/api/logs/${log.id}/unlock`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ password: "pässwörd" }),
  });
  assert.equal(right.status, 204);
  const cookie = right.headers.get("set-cookie").split(";")[0];
  assert.match(right.headers.get("set-cookie"), /HttpOnly/);
  assert.equal((await content(log.id, "", cookie)).text, line(1));

  // The page asks for the password instead of showing the log.
  const page = await (await fetch(`${BASE}/l/${log.id}`)).text();
  assert.match(page, /password protected/);
  assert.doesNotMatch(page, /agent-graph-config/);
});

test("bad input is refused", async () => {
  const log = await create(line(1));
  assert.equal((await append(log.id, "abc", line(2), log.writeToken)).status, 400, "bad offset");
  assert.equal((await append(log.id, 99_999_999_999, line(2), log.writeToken)).status, 413, "past the size limit");
  const big = "x".repeat(600 * 1024);
  assert.equal((await append(log.id, 10, big, log.writeToken)).status, 413, "chunk too big");
  assert.equal((await content("not-an-id")).status, 404);
  assert.equal((await content("AAAAAAAAAAAA")).status, 404);
  assert.equal((await fetch(`${BASE}/l/AAAAAAAAAAAA`)).status, 404);
  const badPassword = await fetch(`${BASE}/api/logs`, {
    method: "POST",
    headers: { "X-Agent-Graph-Password": "not base64url!" },
    body: line(1),
  });
  assert.equal(badPassword.status, 400);
});
