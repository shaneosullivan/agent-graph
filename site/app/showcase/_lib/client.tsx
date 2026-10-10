"use client";

// Calling the Agent Graph API from the browser, by way of the site's proxy
// (app/showcase/api/ag), which adds the demo account's key. Every call is recorded, so each
// page can show exactly what it asked the API for, and how to ask it
// yourself.

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";

import {setNow} from "./format";
import {API_BASES, useSource} from "./source";
import type {ApiErrorBody, Graph, List} from "./types";

export type Query = Record<
  string,
  string | number | boolean | Array<string> | undefined | null
>;

/** One call to the API, as the page's "Under the hood" panel shows it. */
export type Call = {
  id: number;
  path: string;
  status: number;
  ms: number;
  at: number;
  requestId: string | null;
  notModified: boolean;
};

export class AgentGraphError extends Error {
  status: number;
  code: string;
  param: string | null;
  constructor(status: number, body: ApiErrorBody | null) {
    super(body?.error.message ?? `The API answered ${status}.`);
    this.status = status;
    this.code = body?.error.code ?? "unknown";
    this.param = body?.error.param ?? null;
  }
}

/** `path` with `query`: arrays as `name[]=a&name[]=b`, as the API reads them. */
export function withQuery(path: string, query: Query = {}): string {
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(query)) {
    if (value === undefined || value === null || value === "") {
      continue;
    }
    if (Array.isArray(value)) {
      for (const one of value) {
        params.append(`${key}[]`, one);
      }
    } else {
      params.append(key, String(value));
    }
  }
  const qs = params.toString();
  return qs ? `${path}?${qs}` : path;
}

type CallLog = {
  calls: Array<Call>;
  record: (call: Omit<Call, "id">) => void;
  clear: () => void;
};

const CallLogContext = createContext<CallLog | null>(null);

export function CallLogProvider({children}: {children: React.ReactNode}) {
  const [calls, setCalls] = useState<Array<Call>>([]);
  const next = useRef(1);
  const record = useCallback((call: Omit<Call, "id">) => {
    setCalls(c => [{...call, id: next.current++}, ...c].slice(0, 60));
  }, []);
  const clear = useCallback(() => setCalls([]), []);
  const value = useMemo(() => ({calls, record, clear}), [calls, record, clear]);
  return (
    <CallLogContext.Provider value={value}>{children}</CallLogContext.Provider>
  );
}

export function useCallLog(): CallLog {
  const log = useContext(CallLogContext);
  if (!log) {
    throw new Error("useCallLog outside CallLogProvider");
  }
  return log;
}

/**
 * The demo's graphs, each pinned to its last event (its id, and when it
 * was): they don't change, so read as they stood then, nothing in them has
 * gone stale just by being old, and "how long ago" is from then (format.ts
 * `now`). An account's own graphs are read as they are.
 */
const pins = new Map<string, Promise<{id: string; at: number} | null>>();

/** The reads of a graph's state at a moment: the graph, its nodes, a node, a search (not its events). */
const PINNED = /^\/graphs\/([^/]+)(\/nodes(\/[^/]+)?)?$/;

/** The API, as this page calls it: `get` one thing, or `getAll` of a list's pages. */
export function useApi() {
  const {record} = useCallLog();
  const {source} = useSource();

  const get = useCallback(
    // Named, so it can call itself (to pin a graph, below).
    async function get<T>(
      path: string,
      query: Query = {},
      opts: {etag?: string | null} = {},
    ): Promise<{data: T | null; etag: string | null; status: number}> {
      const graph =
        source === "demo" && !("as_of" in query)
          ? PINNED.exec(path)?.[1]
          : undefined;
      if (graph) {
        if (!pins.has(graph)) {
          pins.set(
            graph,
            get<Graph>(`/graphs/${graph}`, {as_of: undefined}).then(
              ({data}) => {
                const r = data?.retention;
                return r?.last_event_id && r.last_event_created
                  ? {id: r.last_event_id, at: r.last_event_created}
                  : null;
              },
              () => null,
            ),
          );
        }
        const pin = await pins.get(graph);
        if (pin) {
          setNow(pin.at);
          query = {...query, as_of: pin.id};
        }
      }
      const full = withQuery(path, query);
      const started = performance.now();
      const res = await fetch(`${API_BASES[source]}${full}`, {
        headers: opts.etag ? {"If-None-Match": opts.etag} : {},
        cache: "no-store",
      });
      const ms =
        Number(res.headers.get("x-upstream-ms")) ||
        Math.round(performance.now() - started);
      record({
        path: full,
        status: res.status,
        ms,
        at: Date.now(),
        requestId: res.headers.get("request-id"),
        notModified: res.status === 304,
      });
      if (res.status === 304) {
        return {data: null, etag: opts.etag ?? null, status: 304};
      }
      const text = await res.text();
      const body = text ? JSON.parse(text) : null;
      if (!res.ok) {
        throw new AgentGraphError(res.status, body);
      }
      return {
        data: body as T,
        etag: res.headers.get("etag"),
        status: res.status,
      };
    },
    [record, source],
  );

  /** Every item of a list, following its cursor, up to `maxPages` pages. */
  const getAll = useCallback(
    async <T extends {id: string}>(
      path: string,
      query: Query = {},
      maxPages = 5,
    ): Promise<{data: Array<T>; asOf: string | null; complete: boolean}> => {
      const out: Array<T> = [];
      let after: string | undefined;
      let asOf: string | null = null;
      for (let page = 0; page < maxPages; page++) {
        const {data}: {data: List<T> | null} = await get<List<T>>(path, {
          limit: 1000,
          ...query,
          // Every page at the moment of the first.
          ...(asOf && !("as_of" in query) && !path.endsWith("/events")
            ? {as_of: asOf}
            : {}),
          ...(after ? {starting_after: after} : {}),
        });
        if (!data) {
          break;
        }
        asOf ??= data.as_of_event_id;
        out.push(...data.data);
        if (!data.has_more || !data.data.length) {
          return {data: out, asOf, complete: true};
        }
        after = data.data[data.data.length - 1].id;
      }
      return {data: out, asOf, complete: false};
    },
    [get],
  );

  return {get, getAll};
}

/** Runs `load` when `deps` change: its result, or what went wrong. */
export function useLoad<T>(
  load: () => Promise<T>,
  deps: ReadonlyArray<unknown>,
): {data: T | null; error: Error | null; loading: boolean; reload: () => void} {
  const [state, setState] = useState<{
    data: T | null;
    error: Error | null;
    loading: boolean;
  }>({
    data: null,
    error: null,
    loading: true,
  });
  const [nonce, setNonce] = useState(0);
  useEffect(() => {
    let live = true;
    // Loading again: what's shown is the last load's until this one's done.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setState(s => ({...s, loading: true, error: null}));
    load().then(
      data => live && setState({data, error: null, loading: false}),
      error => live && setState({data: null, error, loading: false}),
    );
    return () => {
      live = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [...deps, nonce]);
  return {...state, reload: () => setNonce(n => n + 1)};
}
