// Unit tests for lib/encryption.ts:  npm run test:unit

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { test } from "node:test";

process.env.AGENT_GRAPH_ENCRYPTION_KEY = Buffer.alloc(32, 1).toString("base64url");
const { decryptChunk, encryptChunk } = await import("../lib/encryption.ts");

const LOG = "AbCdEf123456";
const CHUNK = "000000000000000";
const TEXT = '{"v":1,"id":"01K","type":"status","data":{"summary":"héllo ✓ 修复"}}\n';

test("round-trips text, including non-ASCII", () => {
  assert.equal(decryptChunk(LOG, CHUNK, encryptChunk(LOG, CHUNK, TEXT)), TEXT);
  assert.equal(decryptChunk(LOG, CHUNK, encryptChunk(LOG, CHUNK, "")), "");
});

test("stores no plaintext, and never the same bytes twice", () => {
  const a = encryptChunk(LOG, CHUNK, TEXT);
  const b = encryptChunk(LOG, CHUNK, TEXT);
  assert.ok(!a.includes(Buffer.from("summary")));
  assert.notDeepEqual(a, b, "a fresh IV each time");
  assert.equal(a.length, 1 + 12 + Buffer.byteLength(TEXT) + 16);
});

test("detects any change to the stored bytes", () => {
  const stored = encryptChunk(LOG, CHUNK, TEXT);
  for (const i of [1, 13, stored.length - 1]) {
    const tampered = Buffer.from(stored);
    tampered[i] ^= 1;
    assert.throws(() => decryptChunk(LOG, CHUNK, tampered), `byte ${i}`);
  }
  assert.throws(() => decryptChunk(LOG, CHUNK, stored.subarray(0, 20)), "truncated");
  assert.throws(() => decryptChunk(LOG, CHUNK, Buffer.concat([Buffer.of(9), stored.subarray(1)])), "unknown version");
});

test("a chunk can't be moved to another place or another log", () => {
  const stored = encryptChunk(LOG, CHUNK, TEXT);
  assert.throws(() => decryptChunk(LOG, "000000000000100", stored), "another offset");
  assert.throws(() => decryptChunk("ZZZZZZZZZZZZ", CHUNK, stored), "another log");
});

test("another master key can't read it", () => {
  const stored = encryptChunk(LOG, CHUNK, TEXT).toString("base64");
  const other = spawnSync(
    process.execPath,
    [
      "--experimental-strip-types",
      "--no-warnings",
      "--input-type=module",
      "-e",
      `const { decryptChunk } = await import(${JSON.stringify(new URL("../lib/encryption.ts", import.meta.url).href)});
       decryptChunk(${JSON.stringify(LOG)}, ${JSON.stringify(CHUNK)}, Buffer.from(${JSON.stringify(stored)}, "base64"));`,
    ],
    { env: { ...process.env, AGENT_GRAPH_ENCRYPTION_KEY: Buffer.alloc(32, 2).toString("base64url") } },
  );
  assert.notEqual(other.status, 0);
});

test("rejects a master key of the wrong size", () => {
  const bad = spawnSync(
    process.execPath,
    [
      "--experimental-strip-types",
      "--no-warnings",
      "--input-type=module",
      "-e",
      `const { encryptChunk } = await import(${JSON.stringify(new URL("../lib/encryption.ts", import.meta.url).href)});
       encryptChunk("x", "0", "text");`,
    ],
    { env: { ...process.env, AGENT_GRAPH_ENCRYPTION_KEY: "too-short" }, encoding: "utf8" },
  );
  assert.notEqual(bad.status, 0);
  assert.match(bad.stderr, /must be 32 bytes/);
});
