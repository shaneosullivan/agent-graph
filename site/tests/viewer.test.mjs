// The viewer's behaviour, in jsdom. See viewer-harness.mjs.

import assert from "node:assert/strict";
import { test } from "node:test";

import { graph, loadViewer, node, until } from "./viewer-harness.mjs";

test("R3: a tree with a loop in it is drawn once, and the page keeps working", async (t) => {
  const a = node("x:a", { children: ["x:b"] });
  const b = node("x:b", { parent: "x:a", children: ["x:a"] });
  const g = graph([a, b], { roots: ["x:a"] });
  const window = loadViewer(t, { graph: async () => g });
  const doc = window.document;
  await until(() => doc.querySelectorAll("#view .node").length || !doc.querySelector("#banner").hidden);
  assert.equal(doc.querySelector("#banner").hidden, true, doc.querySelector("#banner").textContent);
  assert.equal(doc.querySelectorAll("#view .node").length, 2);
});
