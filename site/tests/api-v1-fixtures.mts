// Logs for the graph API's tests (tests/api-v1*.test.mts): the repository's
// example logs (examples/logs/events), cut to start from a keyframe as a
// live share's is, and small ones made to order.

import {readdirSync, readFileSync} from "node:fs";

const EXAMPLES = new URL("../../examples/logs/events/", import.meta.url);

/** Every example log, as one log's text. */
export function examples(): string {
  return readdirSync(EXAMPLES)
    .filter(f => f.endsWith(".jsonl"))
    .sort()
    .map(f => readFileSync(new URL(f, EXAMPLES), "utf8"))
    .join("");
}

const wasm = (
  await WebAssembly.instantiate(
    readFileSync(new URL("../public/viewer/agent_graph.wasm", import.meta.url)),
    {},
  )
).instance.exports as unknown as {
  memory: WebAssembly.Memory;
  alloc(n: number): number;
  dealloc(p: number, n: number): void;
  trim_start(p: number, n: number, every: number): void;
  trim_step(lines: number): number;
  trim_finish(): number;
  result_len(): number;
};

/** `text` cut as a live share sends it: its last two keyframes' worth, a keyframe every `every` events. */
export function trimmed(text: string, every: number): string {
  const data = new TextEncoder().encode(text);
  const ptr = wasm.alloc(data.length);
  new Uint8Array(wasm.memory.buffer, ptr, data.length).set(data);
  wasm.trim_start(ptr, data.length, every);
  wasm.dealloc(ptr, data.length);
  while (!wasm.trim_step(1000)) {
    // until it's done
  }
  const out = wasm.trim_finish();
  const n = wasm.result_len();
  const result = new TextDecoder().decode(
    new Uint8Array(wasm.memory.buffer, out, n),
  );
  wasm.dealloc(out, n);
  return result.slice(result.indexOf("\n") + 1);
}

let n = 0;

/** An event's line, at second `at` of 2026-09-25 10:00. */
export function line(
  at: number,
  node: string,
  type: string,
  data: Record<string, unknown> = {},
  extra: Record<string, unknown> = {},
): string {
  n += 1;
  const id = `01K${String(at).padStart(6, "0")}${String(n).padStart(17, "0")}`;
  const ts = new Date(Date.UTC(2026, 8, 25, 10, 0, at)).toISOString();
  return (
    JSON.stringify({
      v: 1,
      id,
      ts,
      type,
      node,
      source: {provider: node.split(":")[0], adapter: "tests"},
      data,
      ...extra,
    }) + "\n"
  );
}

/** Splits `text` into chunks of whole lines, each about `size` bytes, as they'd be stored. */
export function chunks(text: string, size: number): Array<string> {
  const out: Array<string> = [];
  let current = "";
  for (const l of text.split(/(?<=\n)/)) {
    current += l;
    if (Buffer.byteLength(current) >= size) {
      out.push(current);
      current = "";
    }
  }
  if (current) {
    out.push(current);
  }
  return out;
}
