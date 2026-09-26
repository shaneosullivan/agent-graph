// Unit tests for lib/unlock.ts (the unlock route), the addresses it
// counts guesses by, and how the API routes read bodies:  npm run test:unit

import assert from "node:assert/strict";
import { register } from "node:module";
import { test } from "node:test";

register("../scripts/resolve-ts.mjs", import.meta.url);
const { unlock } = await import("../lib/unlock.ts");
const { addressBlock, bodyText } = await import("../lib/config.ts");
type Deps = import("../lib/unlock.ts").UnlockDeps;

const ID = "AbCdEf123456";

/** Fakes for what unlocking uses, recording the calls that matter. */
function fakes(over: Partial<Deps> = {}) {
  const calls: string[] = [];
  const addresses: (string | null)[] = [];
  const deps = {
    getMeta: async () => ({ source: "watch", pw: "the-hash" }) as Awaited<ReturnType<Deps["getMeta"]>>,
    takeUnlockAttempt: async (_id: string, address: string | null) => {
      calls.push("take");
      addresses.push(address);
      return { reservation: [] };
    },
    giveBackUnlockAttempt: async () => {
      calls.push("give back");
    },
    verifyPassword: async (password: string) => {
      calls.push("check");
      return password === "right";
    },
    ...over,
  } as Deps;
  return { deps, calls, addresses };
}

function request(password: string, headers: Record<string, string> = {}) {
  return new Request(`https://site.test/api/logs/${ID}/unlock`, {
    method: "POST",
    headers: { "Content-Type": "application/json", ...headers },
    body: JSON.stringify({ password }),
  });
}

// R19: a refused guess is never checked: checking (scrypt) is the cost the
// limits are there to cap.
test("a refused guess isn't checked", async () => {
  const { deps, calls } = fakes({
    takeUnlockAttempt: async () => {
      calls.push("take");
      return { wait: 90 };
    },
  });
  const res = await unlock(request("right"), ID, deps);
  assert.equal(res.status, 429);
  assert.equal(res.headers.get("retry-after"), "90");
  assert.equal(await res.text(), "Too many tries. Try again in 2 minutes.");
  assert.deepEqual(calls, ["take"]);

  // Busy for a moment: said so.
  const { deps: busy } = fakes({ takeUnlockAttempt: async () => ({ wait: 5 }) });
  const moment = await unlock(request("right"), ID, busy);
  assert.equal(moment.headers.get("retry-after"), "5");
  assert.equal(await moment.text(), "Too many tries. Try again in a few seconds.");
});

// R19: a guess is counted before it's checked (so guesses made at once all
// count), and a right one is given back (so only wrong ones use up the limits).
test("a guess is counted, then checked, and a right one given back", async () => {
  const right = fakes();
  const ok = await unlock(request("right"), ID, right.deps);
  assert.equal(ok.status, 204);
  assert.match(ok.headers.get("set-cookie") ?? "", /HttpOnly/);
  assert.deepEqual(right.calls, ["take", "check", "give back"]);

  const wrong = fakes();
  assert.equal((await unlock(request("wrong"), ID, wrong.deps)).status, 401);
  assert.deepEqual(wrong.calls, ["take", "check"]);
});

test("a password that can't be right isn't counted or checked", async () => {
  for (const password of ["", "x".repeat(1025)]) {
    const { deps, calls } = fakes();
    assert.equal((await unlock(request(password), ID, deps)).status, 401);
    assert.deepEqual(calls, []);
  }
});

// R19: guesses are counted by the address the platform reports, IPv6 by its
// /64; without one, only per log.
test("guesses are counted by the platform's address, by block", async () => {
  const cases: [Record<string, string>, string | null][] = [
    [{ "X-Real-IP": "203.0.113.7" }, "203.0.113.7"],
    [{ "X-Real-IP": "2001:db8:1:2:3:4:5:6" }, "2001:db8:1:2::/64"],
    [{ "X-Forwarded-For": "198.51.100.1, 10.0.0.1" }, "198.51.100.1"],
    [{}, null],
  ];
  for (const [headers, expected] of cases) {
    const { deps, addresses } = fakes();
    await unlock(request("right", headers), ID, deps);
    assert.deepEqual(addresses, [expected], JSON.stringify(headers));
  }
});

test("an IPv6 address is counted by its /64", () => {
  assert.equal(addressBlock("203.0.113.7"), "203.0.113.7");
  assert.equal(addressBlock("::ffff:203.0.113.7"), "203.0.113.7");
  assert.equal(addressBlock("2001:0db8:0001:0002:0003:0004:0005:0006"), "2001:db8:1:2::/64");
  assert.equal(addressBlock("2001:db8:1:2:ffff::9"), "2001:db8:1:2::/64", "same /64");
  assert.equal(addressBlock("2001:db8::1"), "2001:db8:0:0::/64");
  assert.equal(addressBlock("[2001:db8::1]:443"), "2001:db8:0:0::/64");
  assert.equal(addressBlock("fe80::1%eth0"), "fe80:0:0:0::/64");
  assert.equal(addressBlock("::1"), "0:0:0:0::/64");
  assert.equal(addressBlock("203.0.113.7:5678"), "203.0.113.7", "IPv4 with a port");
  assert.equal(addressBlock("::ffff:c000:201"), "192.0.2.1", "IPv4 mapped, in hex");
  assert.equal(addressBlock("0:0:0:0:0:ffff:c000:201"), "192.0.2.1", "written out");
  assert.equal(addressBlock("0:0:0:0:0:FFFF:203.0.113.7"), "203.0.113.7");
  assert.equal(addressBlock("[::ffff:c000:201]:80"), "192.0.2.1");
});

/**
 * A request whose body is `prefix`, then at least `total` bytes of "x" in
 * 64 KiB pieces, sent without a Content-Length (as a chunked upload is);
 * `pulled()` is how many of those have been read from it.
 */
function streamed(total: number, prefix = "") {
  let sent = 0;
  const piece = new Uint8Array(64 * 1024).fill("x".charCodeAt(0));
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      if (prefix) controller.enqueue(new TextEncoder().encode(prefix));
    },
    pull(controller) {
      if (sent >= total) return controller.close();
      sent += piece.length;
      controller.enqueue(piece);
    },
  });
  const req = new Request(`https://site.test/api/logs/${ID}/unlock`, {
    method: "POST",
    body,
    duplex: "half",
  } as RequestInit);
  assert.equal(req.headers.get("content-length"), null);
  return { req, pulled: () => sent };
}

// R45: a body without a Content-Length is read only as far as its limit,
// not buffered whole and measured afterwards.
test("a body is read only as far as its limit", async () => {
  const small = streamed(100 * 1024);
  assert.equal(await bodyText(small.req, 512 * 1024), "x".repeat(small.pulled()));
  const big = streamed(64 * 1024 * 1024);
  assert.equal(await bodyText(big.req, 512 * 1024), null);
  assert.ok(big.pulled() <= 1024 * 1024, `${big.pulled()} bytes read`);
  // A byte-order mark is kept (req.text() drops one).
  const bom = new Request("https://site.test/", { method: "POST", body: "\uFEFFx" });
  assert.equal(await bodyText(bom, 10), "\uFEFFx");
  assert.equal(await bodyText(new Request("https://site.test/", { method: "POST", body: "1234" }), 4), "1234");
  assert.equal(await bodyText(new Request("https://site.test/", { method: "POST", body: "12345" }), 4), null);
  assert.equal(await bodyText(new Request("https://site.test/", { method: "POST" }), 4), "");
});

// R45: the same goes for unlocking, whose body is only ever small.
test("an unlock body is read only as far as its limit", async () => {
  const { deps, calls } = fakes();
  const big = streamed(64 * 1024 * 1024, '{"password":"');
  const res = await unlock(big.req, ID, deps);
  assert.equal(res.status, 413);
  assert.ok(big.pulled() <= 1024 * 1024, `${big.pulled()} bytes read`);
  assert.deepEqual(calls, []);
});
