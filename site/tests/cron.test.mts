// The cleanup cron's check that a request is Vercel Cron's (lib/cleanup.ts):
// npm run test:unit

import assert from "node:assert/strict";
import { register } from "node:module";
import { test } from "node:test";

register("../scripts/resolve-ts.mjs", import.meta.url);
const { cronAllowed } = await import("../lib/cleanup.ts");

const req = (auth: string | null) =>
  new Request("https://example.test/api/cron/cleanup", {
    headers: auth === null ? {} : { Authorization: auth },
  });

test("only Vercel Cron, with the secret, can run the cleanup", () => {
  assert.equal(cronAllowed(req("Bearer the-secret"), "the-secret"), true);
  assert.equal(cronAllowed(req("Bearer nope"), "the-secret"), false);
  assert.equal(cronAllowed(req("the-secret"), "the-secret"), false);
  assert.equal(cronAllowed(req(null), "the-secret"), false);
  // Without the secret set, nothing can: not even "Bearer " or "Bearer undefined".
  for (const auth of ["Bearer ", "Bearer undefined", null]) {
    assert.equal(cronAllowed(req(auth), undefined), false, String(auth));
    assert.equal(cronAllowed(req(auth), ""), false, String(auth));
  }
});
