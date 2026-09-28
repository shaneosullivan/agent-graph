// Unit tests for accounts' pure helpers (lib/config.ts):  npm run test:unit

import assert from "node:assert/strict";
import { test } from "node:test";

const { safeNext } = await import("../lib/config.ts");

test("after logging in, only a path on this site is gone to", () => {
  for (const path of ["/watch", "/account", "/docs#watch-remote", "/l/AbCdEf123456?x=1"]) {
    assert.equal(safeNext(path), path);
  }
  const elsewhere = [
    null,
    undefined,
    "",
    "https://evil.example",
    "//evil.example",
    "/\\evil.example",
    "javascript:alert(1)",
    "watch",
    "/\twatch",
    "/watch\n",
    "/a\u0000b",
  ];
  for (const next of elsewhere) {
    assert.equal(safeNext(next), "/account", JSON.stringify(next));
    assert.equal(safeNext(next, "/watch"), "/watch");
  }
});
