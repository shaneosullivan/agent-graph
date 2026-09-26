/**
 * A worker that cuts a log to share (lib/trim-core.ts), so a big one doesn't
 * hold up the page: it's sent `{ text }`, and sends back `{ progress }` (0
 * to 1) as it goes, then `{ done: { text, events, of } }`, or `{ error }`.
 */

import { runTrim, type TrimExports } from "./trim-core";

export type TrimMessage = { progress: number } | { done: import("./trim-core").Trimmed } | { error: string };

/** Handles one request, posting what it says with `post`. */
export async function handle(
  text: string,
  post: (message: TrimMessage) => void,
  load: () => Promise<TrimExports> = loadWasm,
): Promise<void> {
  try {
    const wasm = await load();
    post({ done: runTrim(wasm, text, (progress) => post({ progress })) });
  } catch (err) {
    post({ error: (err as Error).message || "Couldn't prepare the log." });
  }
}

async function loadWasm(): Promise<TrimExports> {
  const bytes = await (await fetch("/viewer/agent_graph.wasm")).arrayBuffer();
  return (await WebAssembly.instantiate(bytes, {})).instance.exports as unknown as TrimExports;
}

// In a worker (not where this is imported to be tested).
const scope = globalThis as unknown as {
  window?: unknown;
  postMessage?: (message: TrimMessage) => void;
  onmessage?: (e: { data: { text: string } }) => void;
};
if (typeof scope.window === "undefined" && typeof scope.postMessage === "function" && "importScripts" in scope) {
  scope.onmessage = (e) => void handle(e.data.text, (m) => scope.postMessage!(m));
}
