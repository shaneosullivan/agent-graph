// The site's API, end to end, against a running copy of the site.
//
//   npm run emulators            # in one terminal
//   npm run dev                  # in another
//   BASE_URL=http://localhost:3000 npm run test:api

import assert from "node:assert/strict";
import {test} from "node:test";

const BASE = (process.env.BASE_URL || "http://localhost:3000").replace(
  /\/$/,
  "",
);

const line = (n, state = "working") =>
  JSON.stringify({
    v: 1,
    id: `01K0000000000000000000${String(n).padStart(4, "0")}`,
    ts: "2026-09-25T10:00:00.000Z",
    type: "status",
    node: "x:s",
    data: {state},
  }) + "\n";

const b64url = s => Buffer.from(s, "utf8").toString("base64url");

async function create(body, headers = {}) {
  const res = await fetch(`${BASE}/api/logs`, {
    method: "POST",
    headers: {"Content-Type": "application/x-ndjson", ...headers},
    body,
  });
  assert.equal(res.status, 201, await res.clone().text());
  return res.json();
}

function append(id, offset, body, key) {
  return fetch(`${BASE}/api/logs/${id}/append?offset=${offset}`, {
    method: "POST",
    headers: {
      "Content-Type": "application/x-ndjson",
      ...(key ? {Authorization: `Bearer ${key}`} : {}),
    },
    body,
  });
}

async function content(id, after = "", cookie) {
  const res = await fetch(`${BASE}/api/logs/${id}/content?after=${after}`, {
    headers: cookie ? {Cookie: cookie} : {},
  });
  return {
    status: res.status,
    text: await res.text(),
    first: res.headers.get("x-first-chunk"),
    last: res.headers.get("x-last-chunk"),
    more: res.headers.get("x-more") === "1",
  };
}

function trim(id, before, key) {
  return fetch(`${BASE}/api/logs/${id}/trim?before=${before}`, {
    method: "POST",
    headers: key ? {Authorization: `Bearer ${key}`} : {},
  });
}

test("create, append with the key, and read back in order", async () => {
  const first = line(1) + line(2);
  const log = await create(first, {"X-Agent-Graph-Source": "upload"});
  assert.match(log.id, /^[A-Za-z0-9]{12}$/);
  assert.equal(log.url, `${BASE}/l/${log.id}`);

  assert.equal(
    (await append(log.id, first.length, line(3), log.writeToken)).status,
    204,
  );
  const all = await content(log.id);
  assert.equal(all.status, 200);
  assert.equal(all.text, first + line(3));

  // Reading on from the last chunk gets only what's new.
  assert.equal((await content(log.id, all.last)).text, "");
  assert.equal(
    (await append(log.id, (first + line(3)).length, line(4), log.writeToken))
      .status,
    204,
  );
  assert.equal((await content(log.id, all.last)).text, line(4));
});

test("updates need the key the log was created with", async () => {
  const mine = await create(line(1));
  const theirs = await create(line(1));
  assert.equal((await append(mine.id, 100, line(9))).status, 401, "no key");
  assert.equal(
    (await append(mine.id, 100, line(9), "guess")).status,
    401,
    "wrong key",
  );
  assert.equal(
    (await append(mine.id, 100, line(9), theirs.writeToken)).status,
    401,
    "another log's key",
  );
  assert.equal((await content(mine.id)).text, line(1), "nothing was added");
});

test("retrying a chunk doesn't duplicate it", async () => {
  const log = await create(line(1));
  const offset = line(1).length;
  assert.equal(
    (await append(log.id, offset, line(2), log.writeToken)).status,
    204,
  );
  assert.equal(
    (await append(log.id, offset, line(2), log.writeToken)).status,
    204,
    "the same bytes again",
  );
  assert.equal((await content(log.id)).text, line(1) + line(2));
});

// R16: a viewer that has read a chunk never reads it again, so a chunk must
// never change: different bytes at an offset already stored are refused,
// whichever copy arrives first.
test("a chunk is never replaced with different bytes", async () => {
  const log = await create(line(1));
  const offset = line(1).length;
  assert.equal(
    (await append(log.id, offset, line(2), log.writeToken)).status,
    204,
  );
  assert.equal(
    (await append(log.id, offset, line(2) + line(3), log.writeToken)).status,
    409,
    "bigger",
  );
  assert.equal(
    (await append(log.id, offset, line(9), log.writeToken)).status,
    409,
    "other bytes",
  );
  assert.equal((await content(log.id)).text, line(1) + line(2));

  // The first copy wins even when it's the bigger one (a late original).
  const other = await create(line(1));
  assert.equal(
    (await append(other.id, offset, line(2) + line(3), other.writeToken))
      .status,
    204,
  );
  assert.equal(
    (await append(other.id, offset, line(2), other.writeToken)).status,
    409,
  );
  assert.equal((await content(other.id)).text, line(1) + line(2) + line(3));
});

// R17: one read returns a bounded number of bytes, however big the chunks,
// and the reader pages through the rest.
test("a read stops at a byte budget, and paging gets the rest", async () => {
  const big = n =>
    line(n).replace(
      '"working"',
      `"working","summary":"${"x".repeat(400_000)}"`,
    );
  const parts = Array.from({length: 8}, (_, i) => big(i + 1));
  const log = await create(parts[0], {"X-Agent-Graph-Source": "upload"});
  let offset = Buffer.byteLength(parts[0]);
  for (const part of parts.slice(1)) {
    assert.equal(
      (await append(log.id, offset, part, log.writeToken)).status,
      204,
    );
    offset += Buffer.byteLength(part);
  }

  const first = await content(log.id);
  assert.equal(first.status, 200);
  assert.ok(
    Buffer.byteLength(first.text) <= 2.5 * 1024 * 1024,
    `${Buffer.byteLength(first.text)} bytes in one read`,
  );
  assert.ok(first.more, "says there's more");

  let text = first.text;
  let page = first;
  while (page.more) {
    page = await content(log.id, page.last);
    assert.ok(Buffer.byteLength(page.text) <= 2.5 * 1024 * 1024);
    text += page.text;
  }
  assert.equal(text, parts.join(""));

  // Chunks whose offsets overlap (only a log's own writer could send them)
  // are refused, so they're no way round it.
  const crafted = await create(parts[0]);
  assert.equal(
    (await append(crafted.id, 1, parts[1], crafted.writeToken)).status,
    409,
  );
});

// R16: a late original racing its retry: every copy is accepted, once.
test("the same chunk sent several times at once is stored once", async () => {
  const log = await create(line(1));
  const offset = line(1).length;
  const statuses = await Promise.all(
    Array.from({length: 6}, () =>
      append(log.id, offset, line(2), log.writeToken).then(r => r.status),
    ),
  );
  assert.deepEqual(statuses, Array(6).fill(204));
  assert.equal((await content(log.id)).text, line(1) + line(2));
});

test("password-protected logs need the password to read", async () => {
  const log = await create(line(1), {
    "X-Agent-Graph-Password": b64url("pässwörd"),
  });
  assert.equal((await content(log.id)).status, 401);

  const wrong = await fetch(`${BASE}/api/logs/${log.id}/unlock`, {
    method: "POST",
    headers: {"Content-Type": "application/json"},
    body: JSON.stringify({password: "nope"}),
  });
  assert.equal(wrong.status, 401);

  const right = await fetch(`${BASE}/api/logs/${log.id}/unlock`, {
    method: "POST",
    headers: {"Content-Type": "application/json"},
    body: JSON.stringify({password: "pässwörd"}),
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

/** Tries a password on a log, from `address` (as the platform reports it). */
function unlock(id, password, address) {
  return fetch(`${BASE}/api/logs/${id}/unlock`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      ...(address ? {"X-Real-IP": address} : {}),
    },
    body: JSON.stringify({password}),
  });
}

/** A random address, so runs against the same emulator don't meet each other's counts. */
const anAddress = () =>
  `10.${[0, 0, 0].map(() => Math.floor(Math.random() * 256)).join(".")}`;

const protectedLog = () =>
  create(line(1), {"X-Agent-Graph-Password": b64url("pässwörd")});

// R19: wrong guesses at a log's password are limited, however many
// addresses they come from; while it cools down, even the right one waits.
test("unlocking a log is limited, from any number of addresses", async () => {
  const log = await protectedLog();
  for (let i = 0; i < 20; i++)
    assert.equal((await unlock(log.id, "nope", anAddress())).status, 401);
  const refused = await unlock(log.id, "pässwörd", anAddress());
  assert.equal(refused.status, 429);
  assert.ok(
    Number(refused.headers.get("retry-after")) > 0,
    "says when to try again",
  );
  assert.match(await refused.text(), /Too many tries/);

  const other = await protectedLog();
  assert.equal(
    (await unlock(other.id, "pässwörd", anAddress())).status,
    204,
    "other logs aren't affected",
  );
});

// R19: one address alone can't lock a log for everyone else.
test("one address's guesses at a log hold back only that address", async () => {
  const log = await protectedLog();
  const guesser = anAddress();
  for (let i = 0; i < 5; i++)
    assert.equal((await unlock(log.id, "nope", guesser)).status, 401);
  assert.equal((await unlock(log.id, "pässwörd", guesser)).status, 429);
  assert.equal(
    (await unlock(log.id, "pässwörd", anAddress())).status,
    204,
    "others still get in",
  );
});

// R19: one address can't guess at many logs either.
test("unlocking is limited per address, across logs", async () => {
  const logs = [];
  for (let i = 0; i < 7; i++) logs.push(await protectedLog());
  const guesser = anAddress();
  for (const log of logs.slice(0, 6)) {
    for (let i = 0; i < 5; i++)
      assert.equal((await unlock(log.id, "nope", guesser)).status, 401);
  }
  assert.equal(
    (await unlock(logs[6].id, "pässwörd", guesser)).status,
    429,
    "a log it hasn't tried yet",
  );
  assert.equal(
    (await unlock(logs[6].id, "pässwörd", anAddress())).status,
    204,
    "other addresses aren't affected",
  );
});

// R19: only wrong guesses count: a log shared with many people, or many
// people behind one address, isn't locked by their getting it right.
test("right passwords don't use up the limits", async () => {
  const log = await protectedLog();
  for (let i = 0; i < 25; i++)
    assert.equal((await unlock(log.id, "pässwörd", anAddress())).status, 204);
  const office = anAddress();
  for (let i = 0; i < 8; i++)
    assert.equal((await unlock(log.id, "pässwörd", office)).status, 204);
  // And wrong ones still count in full.
  for (let i = 0; i < 20; i++)
    assert.equal((await unlock(log.id, "nope", anAddress())).status, 401);
  assert.equal((await unlock(log.id, "pässwörd", anAddress())).status, 429);
});

// R52: every scrypt run is counted, right passwords too (each costs about
// 50 ms of the server's time): 200 checks at a log from one address every 15
// minutes, so one log's viewers can't hold the address back from others.
// (The per-address limit on all scrypt runs, 2000, is tested against the
// store: store.test.mjs.)
test(
  "checking passwords at a log is limited per address",
  {timeout: 120_000},
  async () => {
    const address = anAddress();
    const withPassword = {"X-Agent-Graph-Password": b64url("pässwörd")};
    const log = await create(line(1), {...withPassword, "X-Real-IP": address});
    const other = await create(line(1), {
      ...withPassword,
      "X-Real-IP": address,
    });
    for (let i = 0; i < 200; i++)
      assert.equal(
        (await unlock(log.id, "pässwörd", address)).status,
        204,
        `unlock ${i}`,
      );
    const refused = await unlock(log.id, "pässwörd", address);
    assert.equal(refused.status, 429);
    assert.ok(
      Number(refused.headers.get("retry-after")) > 60,
      "until the window ends",
    );
    assert.equal(
      (await unlock(other.id, "pässwörd", address)).status,
      204,
      "other logs aren't affected",
    );
    assert.equal(
      (await unlock(log.id, "pässwörd", anAddress())).status,
      204,
      "nor other addresses",
    );
    await create(line(1), {...withPassword, "X-Real-IP": address});
  },
);

// Keyframes: a live share keeps only its last two keyframes' worth.
test("a log's start can be trimmed, with its key", async () => {
  const log = await create(line(1));
  const offsets = [0, line(1).length, line(1).length + line(2).length];
  assert.equal(
    (await append(log.id, offsets[1], line(2), log.writeToken)).status,
    204,
  );
  assert.equal(
    (await append(log.id, offsets[2], line(3), log.writeToken)).status,
    204,
  );
  assert.equal((await content(log.id)).first, "000000000000000");

  assert.equal((await trim(log.id, offsets[2])).status, 401, "no key");
  assert.equal(
    (await trim(log.id, offsets[2], "nope")).status,
    401,
    "wrong key",
  );
  assert.equal((await trim(log.id, "abc", log.writeToken)).status, 400);
  assert.equal((await trim("not-an-id", 1, log.writeToken)).status, 404);
  assert.equal(
    (await content(log.id)).text,
    line(1) + line(2) + line(3),
    "nothing gone",
  );

  assert.equal((await trim(log.id, offsets[2], log.writeToken)).status, 204);
  const read = await content(log.id);
  assert.equal(read.text, line(3));
  assert.equal(read.first, String(offsets[2]).padStart(15, "0"));
});

// R44: each chunk starts where the one before it ends, so what a log stores
// is the span from its first chunk to its last, and that's what's limited.
test("a chunk must start where the log ends", async () => {
  const log = await create(line(1));
  const end = line(1).length;
  const refused = await append(log.id, end + 1, line(2), log.writeToken);
  assert.equal(refused.status, 409, "a gap");
  assert.match(await refused.text(), /where the log's last chunk ends/);
  assert.equal(
    (await append(log.id, end - 1, line(2), log.writeToken)).status,
    409,
    "an overlap",
  );
  assert.equal(
    (await append(log.id, end, line(2), log.writeToken)).status,
    204,
  );
  assert.equal((await content(log.id)).text, line(1) + line(2));
});

// What's stored counts towards a log's size limit, not what was ever sent:
// a trimmed live share can go on for good.
test(
  "a log is limited in size, and trimmed, it can go on",
  {timeout: 180_000},
  async () => {
    const big = `${"x".repeat(512 * 1024 - 1)}\n`;
    const size = Buffer.byteLength(big);
    const log = await create(big);
    // 128 of them fill 64 MiB; one more doesn't fit.
    let offset = size;
    for (let i = 1; i < 128; i++, offset += size)
      assert.equal(
        (await append(log.id, offset, big, log.writeToken)).status,
        204,
        `chunk ${i}`,
      );
    assert.equal(
      (await append(log.id, offset, big, log.writeToken)).status,
      204,
      "up to the limit",
    );
    offset += size;
    const full = await append(log.id, offset, big, log.writeToken);
    assert.equal(full.status, 413);
    assert.match(await full.text(), /full/);
    // Trimming makes room again.
    assert.equal((await trim(log.id, 100 * size, log.writeToken)).status, 204);
    assert.equal(
      (await append(log.id, offset, big, log.writeToken)).status,
      204,
    );
    assert.equal(
      (await content(log.id)).first,
      String(100 * size).padStart(15, "0"),
    );
    // But not before where it now starts, nor once it's all gone.
    assert.equal(
      (await append(log.id, 99 * size, big, log.writeToken)).status,
      409,
    );
    assert.equal(
      (await trim(log.id, offset + size + 1, log.writeToken)).status,
      204,
    );
    assert.equal(
      (await append(log.id, offset + size, big, log.writeToken)).status,
      413,
    );
  },
);

// Stored as sent, so chunks' offsets stay true: a byte-order mark at a
// chunk's start (Windows PowerShell writes them) is kept.
test("a chunk is stored as it was sent, a byte-order mark and all", async () => {
  const bom = "\uFEFF" + line(2);
  const log = await create(line(1));
  assert.equal(
    (await append(log.id, line(1).length, bom, log.writeToken)).status,
    204,
  );
  const next = line(1).length + Buffer.byteLength(bom);
  assert.equal(
    (await append(log.id, next, line(3), log.writeToken)).status,
    204,
  );
  const read = await content(log.id);
  assert.deepEqual(
    [read.text, read.more],
    [line(1) + bom + line(3), false],
    "all of it, with no gap",
  );
});

// The daily cron that deletes logs with no event for a week (lib/cleanup.ts)
// runs only for Vercel Cron, which sends the secret.
test("the cleanup cron needs its secret", async () => {
  const run = auth =>
    fetch(`${BASE}/api/cron/cleanup`, {
      headers: auth ? {Authorization: auth} : {},
    });
  assert.equal((await run()).status, 401);
  assert.equal((await run("Bearer nope")).status, 401);
  const secret = process.env.CRON_SECRET;
  assert.ok(
    secret,
    "the tests run with CRON_SECRET set (scripts/ci-api-test.sh)",
  );
  const log = await create(line(1));
  const res = await run(`Bearer ${secret}`);
  assert.equal(res.status, 200);
  const result = await res.json();
  assert.equal(result.done, true);
  assert.ok(result.checked >= 1);
  assert.equal((await content(log.id)).text, line(1), "a log in use is kept");
});

test("bad input is refused", async () => {
  const log = await create(line(1));
  assert.equal(
    (await append(log.id, "abc", line(2), log.writeToken)).status,
    400,
    "bad offset",
  );
  assert.equal(
    (await append(log.id, 99_999_999_999, line(2), log.writeToken)).status,
    413,
    "past the size limit",
  );
  const big = "x".repeat(600 * 1024);
  assert.equal(
    (await append(log.id, 10, big, log.writeToken)).status,
    413,
    "chunk too big",
  );
  assert.equal((await content("not-an-id")).status, 404);
  assert.equal((await content("AAAAAAAAAAAA")).status, 404);
  assert.equal((await fetch(`${BASE}/l/AAAAAAAAAAAA`)).status, 404);
  const badPassword = await fetch(`${BASE}/api/logs`, {
    method: "POST",
    headers: {"X-Agent-Graph-Password": "not base64url!"},
    body: line(1),
  });
  assert.equal(badPassword.status, 400);
  // R46: no longer than unlocking checks.
  const longPassword = await fetch(`${BASE}/api/logs`, {
    method: "POST",
    headers: {"X-Agent-Graph-Password": b64url("x".repeat(1025))},
    body: line(1),
  });
  assert.equal(longPassword.status, 400);
  assert.match(await longPassword.text(), /at most 1024 bytes/);

  // R45: sent without a Content-Length, a body is measured as it's read.
  const streamed = bytes => ({
    method: "POST",
    duplex: "half",
    body: new ReadableStream({
      start(controller) {
        controller.enqueue(
          new TextEncoder().encode(`{"password":"${"x".repeat(bytes)}"}`),
        );
        controller.close();
      },
    }),
  });
  assert.equal(
    (await fetch(`${BASE}/api/logs`, streamed(600 * 1024))).status,
    413,
    "created too big",
  );
  const locked = await create(line(1), {
    "X-Agent-Graph-Password": b64url("pässwörd"),
  });
  assert.equal(
    (await fetch(`${BASE}/api/logs/${locked.id}/unlock`, streamed(100 * 1024)))
      .status,
    413,
  );
});

test(
  "logs are encrypted in Firestore, and not stored under their ids",
  {
    skip:
      !process.env.FIRESTORE_EMULATOR_HOST &&
      "reads the database directly, so needs the emulator",
  },
  async () => {
    const marker = "a very recognisable summary 7c1f";
    const event = n =>
      JSON.stringify({
        v: 1,
        id: `01K0000000000000000000${String(n).padStart(4, "0")}`,
        ts: "2026-09-25T10:00:00.000Z",
        type: "status",
        node: "x:s",
        data: {state: "working", summary: marker},
      }) + "\n";
    const log = await create(event(1));
    assert.equal(
      (
        await append(
          log.id,
          Buffer.byteLength(event(1)),
          event(2),
          log.writeToken,
        )
      ).status,
      204,
    );

    // What's actually stored, read past the site (the emulator's admin token):
    // nothing under the log's id (the link), and nowhere its text. (The
    // chunks' ciphertext is checked more closely in store.test.mjs.)
    const project =
      process.env.GCLOUD_PROJECT ||
      process.env.FIREBASE_PROJECT_ID ||
      "demo-agent-graph";
    const db = `http://${process.env.FIRESTORE_EMULATOR_HOST}/v1/projects/${project}/databases/(default)/documents`;
    const admin = {headers: {Authorization: "Bearer owner"}};
    assert.equal((await fetch(`${db}/logs/${log.id}`, admin)).status, 404);
    const chunks = await (
      await fetch(`${db}/logs/${log.id}/chunks`, admin)
    ).json();
    assert.equal(chunks.documents, undefined, "no chunks under the id");
    const everything = await fetch(`${db}:runQuery`, {
      method: "POST",
      headers: {...admin.headers, "Content-Type": "application/json"},
      body: JSON.stringify({structuredQuery: {from: [{collectionId: "logs"}]}}),
    });
    const listed = await everything.text();
    assert.ok(!listed.includes(log.id), "the id is nowhere in the stored logs");
    assert.doesNotMatch(listed, /recognisable/);

    // The site still reads it back.
    assert.equal((await content(log.id)).text, event(1) + event(2));
  },
);

test("the live build's version, never cached, and a service worker that isn't", async () => {
  const res = await fetch(`${BASE}/version.json?t=${Date.now()}`);
  assert.equal(res.status, 200);
  assert.match(res.headers.get("cache-control") ?? "", /no-store/);
  const {version} = await res.json();
  assert.ok(typeof version === "string" && version.length > 0, version);

  const sw = await fetch(`${BASE}/sw.js`);
  assert.equal(sw.status, 200);
  assert.match(sw.headers.get("cache-control") ?? "", /no-cache/);
  const code = await sw.text();
  // It caches nothing: no cache is ever opened or written to.
  assert.doesNotMatch(code, /caches\.open|cache\.put|cache\.add/);
});
