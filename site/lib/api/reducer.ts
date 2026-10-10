// The reducer, run on the server: the same WebAssembly the viewer runs
// (public/viewer/agent_graph.wasm, from ../wasm), so the API and the viewer
// always agree. Each instance holds one log; see wasm/src/lib.rs for the
// requests it answers.

import {readFile} from "node:fs/promises";
import path from "node:path";

import {apiFailed} from "./errors";
import type {RNodes} from "./model";

/** An event as it was sent, and when it happened (`ms`). */
export type LogEvent = {
  id: string;
  ts: string;
  ms: number;
  type: string;
  node: string;
  parent?: string;
  source?: {provider?: string; provider_version?: string; adapter?: string};
  data?: Record<string, unknown>;
};

type Exports = {
  memory: WebAssembly.Memory;
  alloc(len: number): number;
  dealloc(ptr: number, len: number): void;
  append(ptr: number, len: number): number;
  reset(): void;
  query(ptr: number, len: number): number;
  result_len(): number;
};

let compiled: Promise<WebAssembly.Module> | null = null;

/** The module, compiled once per server instance. */
function module(): Promise<WebAssembly.Module> {
  compiled ??= readFile(
    path.join(process.cwd(), "public", "viewer", "agent_graph.wasm"),
  ).then(bytes => WebAssembly.compile(bytes));
  return compiled;
}

const enc = new TextEncoder();
const dec = new TextDecoder();

/** One log, loaded into its own instance of the reducer. */
export class LogReducer {
  private readonly wasm: Exports;

  private constructor(wasm: Exports) {
    this.wasm = wasm;
  }

  static async create(): Promise<LogReducer> {
    const instance = await WebAssembly.instantiate(await module(), {});
    return new LogReducer(instance.exports as unknown as Exports);
  }

  private put(text: string): [number, number] {
    const data = enc.encode(text);
    const ptr = this.wasm.alloc(data.length);
    new Uint8Array(this.wasm.memory.buffer, ptr, data.length).set(data);
    return [ptr, data.length];
  }

  /** Adds JSON Lines; returns how many new events they held. */
  append(text: string): number {
    if (!text) {
      return 0;
    }
    const [ptr, len] = this.put(text);
    const added = this.wasm.append(ptr, len);
    this.wasm.dealloc(ptr, len);
    return added;
  }

  reset(): void {
    this.wasm.reset();
  }

  private query<T>(request: object): T {
    const [ptr, len] = this.put(JSON.stringify(request));
    const out = this.wasm.query(ptr, len);
    this.wasm.dealloc(ptr, len);
    const n = this.wasm.result_len();
    const text = dec.decode(new Uint8Array(this.wasm.memory.buffer, out, n));
    this.wasm.dealloc(out, n);
    const reply = JSON.parse(text) as T & {status?: number; error?: string};
    if (reply.status !== undefined && reply.error !== undefined) {
      console.error(
        `The reducer refused ${JSON.stringify(request)}: ${reply.error}`,
      );
      throw apiFailed();
    }
    return reply;
  }

  /** The log's events, in the order they're applied; and whether it starts from a keyframe. */
  events(): {base: boolean; events: Array<LogEvent>} {
    return this.query({op: "events"});
  }

  /** Every node after event `until` (or all of them), judged at `nowMs` (or that event's time). */
  state(until?: string, nowMs?: number): {nodes: RNodes; roots: Array<string>} {
    return this.query({
      op: "state",
      ...(until ? {until} : {}),
      ...(nowMs !== undefined ? {now_ms: nowMs} : {}),
    });
  }

  /** The nodes before event `from`, then what each event from `from` to `to` changed. */
  changes(
    from: number,
    to: number,
  ): {
    before: RNodes;
    events: Array<{
      id: string;
      changed: Array<RNodes[string]>;
      removed: Array<string>;
    }>;
  } {
    return this.query({op: "changes", from, to});
  }
}
