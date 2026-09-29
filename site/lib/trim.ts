/**
 * The log to share from some JSON Lines: its last two keyframes' worth
 * (lib/trim-core.ts), cut in a worker (lib/trim-worker.ts) so the page stays
 * responsive while a big one is cut.
 */

import {eventsShared, KEYFRAME_EVERY, type Trimmed} from "./trim-core";
import type {TrimMessage} from "./trim-worker";

export {eventsShared, KEYFRAME_EVERY};

/**
 * `text`, cut to share; `events` is how many it has (by `summarize`, which
 * counts every line the site might read as one, so one too short to cut is
 * surely so, and shared as it is). `onProgress` hears how far along the
 * cutting is, 0 to 1.
 */
export function forSite(
  text: string,
  events: number,
  onProgress?: (done: number) => void,
): Promise<Trimmed> {
  if (eventsShared(events) === events) {
    return Promise.resolve({text, events, of: events});
  }
  return new Promise((resolve, reject) => {
    const worker = new Worker(new URL("./trim-worker.ts", import.meta.url), {
      type: "module",
    });
    worker.onmessage = ({data}: MessageEvent<TrimMessage>) => {
      if ("progress" in data) {
        return onProgress?.(data.progress);
      }
      worker.terminate();
      if ("done" in data) {
        resolve(data.done);
      } else {
        reject(new Error(data.error));
      }
    };
    worker.onerror = e => {
      worker.terminate();
      reject(new Error(e.message || "Couldn't prepare the log."));
    };
    worker.postMessage({text});
  });
}
