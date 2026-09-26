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
  return {
    status: res.status,
    text: await res.text(),
    last: res.headers.get("x-last-chunk"),
    more: res.headers.get("x-more") === "1",
  };
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
  assert.equal((await append(log.id, offset, line(2), log.writeToken)).status, 204);
  assert.equal((await append(log.id, offset, line(2), log.writeToken)).status, 204, "the same bytes again");
  assert.equal((await content(log.id)).text, line(1) + line(2));
});

// R16: a viewer that has read a chunk never reads it again, so a chunk must
// never change: different bytes at an offset already stored are refused,
// whichever copy arrives first.
test("a chunk is never replaced with different bytes", async () => {
  const log = await create(line(1));
  const offset = line(1).length;
  assert.equal((await append(log.id, offset, line(2), log.writeToken)).status, 204);
  assert.equal((await append(log.id, offset, line(2) + line(3), log.writeToken)).status, 409, "bigger");
  assert.equal((await append(log.id, offset, line(9), log.writeToken)).status, 409, "other bytes");
  assert.equal((await content(log.id)).text, line(1) + line(2));

  // The first copy wins even when it's the bigger one (a late original).
  const other = await create(line(1));
  assert.equal((await append(other.id, offset, line(2) + line(3), other.writeToken)).status, 204);
  assert.equal((await append(other.id, offset, line(2), other.writeToken)).status, 409);
  assert.equal((await content(other.id)).text, line(1) + line(2) + line(3));
});

// R17: one read returns a bounded number of bytes, however big the chunks,
// and the reader pages through the rest.
test("a read stops at a byte budget, and paging gets the rest", async () => {
  const big = (n) => line(n).replace('"working"', `"working","summary":"${"x".repeat(400_000)}"`);
  const parts = Array.from({ length: 8 }, (_, i) => big(i + 1));
  const log = await create(parts[0], { "X-Agent-Graph-Source": "watch" });
  let offset = Buffer.byteLength(parts[0]);
  for (const part of parts.slice(1)) {
    assert.equal((await append(log.id, offset, part, log.writeToken)).status, 204);
    offset += Buffer.byteLength(part);
  }

  const first = await content(log.id);
  assert.equal(first.status, 200);
  assert.ok(Buffer.byteLength(first.text) <= 2.5 * 1024 * 1024, `${Buffer.byteLength(first.text)} bytes in one read`);
  assert.ok(first.more, "says there's more");

  let text = first.text;
  let page = first;
  while (page.more) {
    page = await content(log.id, page.last);
    assert.ok(Buffer.byteLength(page.text) <= 2.5 * 1024 * 1024);
    text += page.text;
  }
  assert.equal(text, parts.join(""));

  // Chunks whose offsets overlap (only a log's own writer could store
  // them) are no way round it.
  const crafted = await create(parts[0]);
  for (let i = 1; i < parts.length; i++) {
    assert.equal((await append(crafted.id, i, parts[i], crafted.writeToken)).status, 204);
  }
  const read = await content(crafted.id);
  assert.ok(Buffer.byteLength(read.text) <= 2.5 * 1024 * 1024, `${Buffer.byteLength(read.text)} bytes in one read`);
  assert.ok(read.more);
});

// R16: a late original racing its retry: every copy is accepted, once.
test("the same chunk sent several times at once is stored once", async () => {
  const log = await create(line(1));
  const offset = line(1).length;
  const statuses = await Promise.all(
    Array.from({ length: 6 }, () => append(log.id, offset, line(2), log.writeToken).then((r) => r.status)),
  );
  assert.deepEqual(statuses, Array(6).fill(204));
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
  assert.equal(
    (await append(log.id, 99_999_999_999, line(2), log.writeToken)).status,
    413,
    "past the size limit",
  );
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

test(
  "logs are encrypted in Firestore",
  { skip: !process.env.FIRESTORE_EMULATOR_HOST && "reads the database directly, so needs the emulator" },
  async () => {
    const marker = "a very recognisable summary 7c1f";
    const event = (n) =>
      JSON.stringify({
        v: 1,
        id: `01K0000000000000000000${String(n).padStart(4, "0")}`,
        ts: "2026-09-25T10:00:00.000Z",
        type: "status",
        node: "x:s",
        data: { state: "working", summary: marker },
      }) + "\n";
    const log = await create(event(1));
    assert.equal((await append(log.id, Buffer.byteLength(event(1)), event(2), log.writeToken)).status, 204);

    // What's actually stored, read past the site (the emulator's admin token).
    const project = process.env.GCLOUD_PROJECT || process.env.FIREBASE_PROJECT_ID || "demo-agent-graph";
    const res = await fetch(
      `http://${process.env.FIRESTORE_EMULATOR_HOST}/v1/projects/${project}/databases/(default)/documents/logs/${log.id}/chunks`,
      { headers: { Authorization: "Bearer owner" } },
    );
    assert.equal(res.status, 200);
    const stored = await res.json();
    assert.equal(stored.documents.length, 2);
    for (const doc of stored.documents) {
      assert.deepEqual(Object.keys(doc.fields), ["e"], "only ciphertext");
      const bytes = Buffer.from(doc.fields.e.bytesValue, "base64");
      assert.ok(!bytes.includes(Buffer.from("recognisable")), "no plaintext in the stored bytes");
    }
    assert.doesNotMatch(JSON.stringify(stored), /recognisable/);

    // The site still reads it back.
    assert.equal((await content(log.id)).text, event(1) + event(2));
  },
);

// R16: a stored chunk that can't be decrypted is refused like any other
// mismatch, so the client stops, rather than a 500 it would retry for good.
test(
  "a stored chunk that can't be decrypted isn't replaced, and says so",
  { skip: !process.env.FIRESTORE_EMULATOR_HOST && "writes the database directly, so needs the emulator" },
  async () => {
    const log = await create(line(1));
    const offset = line(1).length;
    assert.equal((await append(log.id, offset, line(2), log.writeToken)).status, 204);

    // Damage it, past the site (the emulator's admin token).
    const project = process.env.GCLOUD_PROJECT || process.env.FIREBASE_PROJECT_ID || "demo-agent-graph";
    const key = String(offset).padStart(15, "0");
    const res = await fetch(
      `http://${process.env.FIRESTORE_EMULATOR_HOST}/v1/projects/${project}/databases/(default)/documents/logs/${log.id}/chunks/${key}?updateMask.fieldPaths=e`,
      {
        method: "PATCH",
        headers: { Authorization: "Bearer owner", "Content-Type": "application/json" },
        body: JSON.stringify({ fields: { e: { bytesValue: Buffer.from("not a chunk").toString("base64") } } }),
      },
    );
    assert.equal(res.status, 200, await res.text());

    assert.equal((await append(log.id, offset, line(2), log.writeToken)).status, 409);
  },
);
