// The home page's examples (lib/examples.json and public/examples/, made by
// scripts/build-examples.mjs):  npm run test:unit

import assert from "node:assert/strict";
import {readFileSync} from "node:fs";
import {test} from "node:test";

const examples: Array<{
  slug: string;
  title: string;
  description: string;
  tags: Array<string>;
  href: string;
}> = JSON.parse(
  readFileSync(new URL("../lib/examples.json", import.meta.url), "utf8"),
);

test("each example has a log, which opens on one of its sessions", () => {
  assert.ok(examples.length >= 8);
  for (const example of examples) {
    const text = readFileSync(
      new URL(`../public/examples/${example.slug}.jsonl`, import.meta.url),
      "utf8",
    );
    const lines = text.split("\n").filter(Boolean);
    assert.ok(lines.length > 0, example.slug);
    const nodes = new Set<string>();
    for (const line of lines) {
      try {
        nodes.add(JSON.parse(line).node);
      } catch {
        // The damaged example's garbage lines, on purpose.
      }
    }
    const url = new URL(example.href, "https://site.test");
    assert.equal(url.pathname, `/examples/${example.slug}`);
    assert.match(url.searchParams.get("view") ?? "", /^(graph|cards)$/);
    const open = decodeURIComponent(url.hash.slice(1));
    assert.ok(nodes.has(open), `${example.slug} opens on ${open}`);
    assert.ok(example.title && example.description && example.tags.length);
  }
});

test("the large example is large, and the busy ones have many sessions", () => {
  const sessions = (slug: string) => {
    const text = readFileSync(
      new URL(`../public/examples/${slug}.jsonl`, import.meta.url),
      "utf8",
    );
    const started = new Set<string>();
    let agents = 0;
    for (const line of text.split("\n").filter(Boolean)) {
      try {
        const e = JSON.parse(line);
        if (e.type === "session.started") started.add(e.node);
        if (e.type === "agent.spawned") agents++;
      } catch {
        // (As above.)
      }
    }
    return {sessions: started.size, agents};
  };
  assert.ok(sessions("large").agents >= 60);
  assert.ok(sessions("team").sessions >= 10);
  assert.ok(sessions("everything").sessions >= 30);
});
