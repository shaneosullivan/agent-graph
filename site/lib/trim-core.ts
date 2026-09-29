/**
 * Cutting a log to share to its last two keyframes' worth, as `agent-graph
 * watch-remote` shares one (src/keyframe.rs), since the site keeps no more:
 * with the site's WebAssembly, a step at a time, saying how far along it is.
 * The page runs it in a worker (lib/trim-worker.ts, lib/trim.ts).
 */

/** Events between keyframes: `keyframe::EVERY` in src/keyframe.rs. */
export const KEYFRAME_EVERY = 1000;

/** How many of a log's `events` are shared: from its last keyframe but one. */
export function eventsShared(events: number): number {
  const keyframes = Math.floor(events / KEYFRAME_EVERY);
  return keyframes >= 2 ? events - (keyframes - 1) * KEYFRAME_EVERY : events;
}

export type Trimmed = {text: string; events: number; of: number};

export type TrimExports = {
  memory: WebAssembly.Memory;
  alloc(len: number): number;
  dealloc(ptr: number, len: number): void;
  trim_start(ptr: number, len: number, every: number): void;
  trim_step(lines: number): number;
  trim_progress(): number;
  trim_finish(): number;
  result_len(): number;
};

/** Lines worked through between reports of how far along it is. */
const STEP = 5000;

/** `text` cut to share; `onProgress` hears how far along it is, 0 to 1. */
export function runTrim(
  wasm: TrimExports,
  text: string,
  onProgress?: (done: number) => void,
): Trimmed {
  const bytes = new TextEncoder().encode(text);
  const ptr = wasm.alloc(bytes.length);
  new Uint8Array(wasm.memory.buffer, ptr, bytes.length).set(bytes);
  wasm.trim_start(ptr, bytes.length, KEYFRAME_EVERY);
  wasm.dealloc(ptr, bytes.length);
  while (!wasm.trim_step(STEP)) {
    onProgress?.(wasm.trim_progress());
  }
  onProgress?.(1);
  const out = wasm.trim_finish();
  const n = wasm.result_len();
  const all = new TextDecoder().decode(
    new Uint8Array(wasm.memory.buffer, out, n),
  );
  wasm.dealloc(out, n);
  const newline = all.indexOf("\n");
  const {events, of} = JSON.parse(all.slice(0, newline));
  return {text: all.slice(newline + 1), events, of};
}
