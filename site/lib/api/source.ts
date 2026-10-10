// A graph's log, loaded into the reducer and kept: each request reads only
// the chunks stored since (one query, usually empty), and what's been
// worked out from it (states, what each event changed) is kept until the
// log changes. A live share is trimmed from its start as it goes, so each
// is read again whole every so often (`FULL_READ_MS`), which also bounds
// what an instance holds.

import {applySteps, type Applied} from "./changes";
import type {LogEvent} from "./reducer";
import {LogReducer} from "./reducer";
import {chunkKey} from "../store";
import {type RNodes, Tree} from "./model";

/** How the log's chunks are read (lib/store.ts `readChunks`). */
export type ReadChunks = (
  id: string,
  after: string,
) => Promise<{
  text: string;
  first: string | null;
  last: string | null;
  more: boolean;
}>;

/** How long a loaded log is carried on with before it's read whole again. */
export const FULL_READ_MS = 10 * 60 * 1000;

/** How many logs an instance keeps loaded. */
const MOST_LOGS = 16;

/** How many states of each it keeps. */
const MOST_STATES = 12;

/** A log, loaded, and what's been worked out from it. */
export class Loaded {
  events: Array<LogEvent> = [];
  /** Event id → its place in `events`. */
  index = new Map<string, number>();
  /** Whether the log starts from a keyframe (its history before isn't here). */
  base = false;
  /** The first chunk read, and where the next should start. */
  first: string | null = null;
  next = 0;
  /** The last chunk read: where to read on from. */
  last: string | null = null;
  readAt = 0;
  private states = new Map<string, Tree>();
  private applied: Array<Applied | undefined> = [];
  readonly logId: string;
  readonly reducer: LogReducer;

  constructor(logId: string, reducer: LogReducer) {
    this.logId = logId;
    this.reducer = reducer;
  }

  /** Re-lists the events after new text was added; keeps what's still true. */
  relist(): void {
    const {base, events} = this.reducer.events();
    // What was worked out for events still in the same places holds.
    let same = 0;
    while (
      same < this.events.length &&
      same < events.length &&
      this.events[same].id === events[same].id
    ) {
      same += 1;
    }
    this.applied.length = Math.min(this.applied.length, same);
    this.events = events;
    this.base = base;
    this.index = new Map(events.map((e, i) => [e.id, i]));
    this.states.clear();
  }

  /**
   * Every node after event `i` (-1: before any), judged at `nowMs` (or, if
   * it isn't given, at that event's time).
   */
  stateAt(i: number, nowMs?: number): Tree {
    const key = `${i}:${nowMs ?? ""}`;
    let tree = this.states.get(key);
    if (!tree) {
      tree =
        i < 0
          ? new Tree({})
          : new Tree(this.reducer.state(this.events[i].id, nowMs).nodes);
      if (this.states.size >= MOST_STATES) {
        this.states.delete(this.states.keys().next().value!);
      }
      this.states.set(key, tree);
    }
    return tree;
  }

  /**
   * Every node after each of events `indexes`, each judged at its own
   * event's time (as `stateAt(i)` is): from one replay, not one each.
   */
  treesAfter(indexes: ReadonlyArray<number>): Map<number, Tree> {
    const out = new Map<number, Tree>();
    if (!indexes.length) {
      return out;
    }
    const from = Math.min(...indexes);
    const {before, events} = this.reducer.changes(from, Math.max(...indexes));
    const wanted = new Set(indexes);
    const nodes: RNodes = {...before};
    events.forEach((step, k) => {
      for (const node of step.changed) {
        nodes[node.id] = node;
      }
      for (const ref of step.removed) {
        delete nodes[ref];
      }
      if (wanted.has(from + k)) {
        out.set(from + k, new Tree({...nodes}));
      }
    });
    return out;
  }

  /** What events `indexes` did (see lib/api/changes.ts), worked out once. */
  appliedAt(indexes: ReadonlyArray<number>, graphId: string): Array<Applied> {
    const missing = indexes.filter(i => !this.applied[i]);
    if (missing.length) {
      // Every event between is worked out too: the reducer replays to the
      // first anyway, and a page's events are near each other.
      const from = Math.min(...missing);
      const to = Math.max(...missing);
      const {before, events} = this.reducer.changes(from, to);
      const subjects = this.events.slice(from, to + 1).map(e => e.node);
      applySteps(before, events, subjects, graphId).forEach((a, k) => {
        this.applied[from + k] = a;
      });
    }
    return indexes.map(i => this.applied[i]!);
  }
}

/**
 * The logs an instance has loaded: `get` reads what's new of one (all of
 * it, the first time, or once it's been a while), then answers from it.
 */
export class Graphs {
  private loaded = new Map<string, Promise<Loaded>>();
  private readonly readChunks: ReadChunks;
  private readonly now: () => number;

  constructor(readChunks: ReadChunks, now: () => number = Date.now) {
    this.readChunks = readChunks;
    this.now = now;
  }

  /** Log `id`, up to date. */
  async get(id: string): Promise<Loaded> {
    const had = this.loaded.get(id);
    const ready = (async () => {
      const prev = had ? await had.catch(() => null) : null;
      if (prev && this.now() - prev.readAt < FULL_READ_MS) {
        await this.readOn(prev);
        return prev;
      }
      return this.readWhole(id);
    })();
    // Requests that come meanwhile wait for this one's read.
    this.loaded.delete(id);
    this.loaded.set(id, ready);
    if (this.loaded.size > MOST_LOGS) {
      this.loaded.delete(this.loaded.keys().next().value!);
    }
    ready.catch(() => {
      if (this.loaded.get(id) === ready) {
        this.loaded.delete(id);
      }
    });
    return ready;
  }

  /** Forgets every log (tests). */
  clear(): void {
    this.loaded.clear();
  }

  // Always a new reducer: a request may still be answering from the one
  // read before, and resetting it under that request would lose its events.
  private async readWhole(id: string): Promise<Loaded> {
    const loaded = new Loaded(id, await LogReducer.create());
    loaded.readAt = this.now();
    await this.readOn(loaded);
    return loaded;
  }

  /**
   * Reads the chunks after the last one read. Chunks follow on from each
   * other but where a log's start was trimmed: if what's read next doesn't
   * start where the last read ended, the log was trimmed while it was read,
   * at a keyframe, so it's read again from there.
   */
  private async readOn(loaded: Loaded): Promise<void> {
    let added = false;
    for (let reads = 0; reads < 10_000; reads++) {
      const {text, first, last, more} = await this.readChunks(
        loaded.logId,
        loaded.last ?? "",
      );
      if (first !== null) {
        if (loaded.first !== null && first !== chunkKey(loaded.next)) {
          loaded.reducer.reset();
          loaded.first = null;
        }
        loaded.first ??= first;
        loaded.next = Number(first) + Buffer.byteLength(text);
        loaded.last = last;
        loaded.reducer.append(text);
        added = true;
      }
      if (!more) {
        break;
      }
    }
    if (added || !loaded.events.length) {
      loaded.relist();
    }
  }
}
