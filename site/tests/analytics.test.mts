// What's counted, and who can see it (lib/analytics-core.ts): npm run test:unit

import assert from "node:assert/strict";
import {register} from "node:module";
import {test} from "node:test";

register("../scripts/resolve-ts.mjs", import.meta.url);
const {analyticsDisabled, countsOf, firstDayKept, isAdmin, periods, TARGETS} =
  await import("../lib/analytics-core.ts");

test("a page view counts the visitor for the day and the month it's new in", () => {
  assert.deepEqual(countsOf({event: "pageview"}), {
    day: ["pageview"],
    month: ["pageview"],
  });
  assert.deepEqual(countsOf({event: "pageview", newDay: true}), {
    day: ["pageview", "visitor"],
    month: ["pageview"],
  });
  assert.deepEqual(
    countsOf({event: "pageview", newDay: true, newMonth: true}),
    {day: ["pageview", "visitor"], month: ["pageview", "visitor"]},
  );
  // Only `true` counts.
  assert.deepEqual(countsOf({event: "pageview", newDay: "yes"})?.day, [
    "pageview",
  ]);
});

test("a download counts for its target, and only a real one", () => {
  for (const target of TARGETS) {
    assert.deepEqual(countsOf({event: "download", target}), {
      day: [`download:${target}`],
      month: [`download:${target}`],
    });
  }
  assert.equal(
    countsOf({event: "download", target: "sparc-sun-solaris"}),
    null,
  );
  assert.equal(countsOf({event: "download"}), null);
  for (const junk of [null, "pageview", 3, [], {event: "signup"}, {}]) {
    assert.equal(countsOf(junk), null, JSON.stringify(junk));
  }
});

test("counting's off only when ANALYTICS_DISABLED is true", () => {
  assert.equal(analyticsDisabled({}), false, "on unless it's said");
  assert.equal(analyticsDisabled({ANALYTICS_DISABLED: "false"}), false);
  assert.equal(analyticsDisabled({ANALYTICS_DISABLED: "true"}), true);
  assert.equal(analyticsDisabled({ANALYTICS_DISABLED: " TRUE "}), true);
  assert.equal(analyticsDisabled({ANALYTICS_DISABLED: "1"}), false);
});

test("periods are UTC days and months; a day's kept for a year", () => {
  assert.deepEqual(periods(new Date("2026-09-29T23:59:59Z")), {
    day: "2026-09-29",
    month: "2026-09",
  });
  assert.equal(firstDayKept(new Date("2026-09-29T12:00:00Z")), "2025-09-29");
});

test("admins are ADMIN_EMAILS' accounts, with their email verified", () => {
  const env = {ADMIN_EMAILS: " Ada@Example.com , ops@example.com,,"};
  const user = (email: string | null, emailVerified = true) => ({
    email,
    emailVerified,
  });
  assert.equal(isAdmin(user("ada@example.com"), env), true, "any case");
  assert.equal(isAdmin(user("OPS@example.com"), env), true);
  assert.equal(isAdmin(user("eve@example.com"), env), false);
  assert.equal(
    isAdmin(user("ada@example.com", false), env),
    false,
    "an unverified email could be anyone's",
  );
  assert.equal(isAdmin(user(null), env), false);
  assert.equal(isAdmin(null, env), false);
  assert.equal(isAdmin(user("ada@example.com"), {}), false, "none set");
});
