// The graph API as the site serves it (app/api/v1/…): each endpoint, with
// what it reads from storage, behind lib/api/handler.ts. One set of loaded
// logs and rate limits per server instance.

import {serve} from "./handler";
import {keyOf} from "./keys";
import {RateLimiter} from "./ratelimit";
import {Graphs} from "./source";
import {resourceMissing} from "./errors";
import * as v1 from "./v1";
import {getMeta, readChunks, sharesOf} from "../store";

const deps: v1.V1Deps = {
  sharesOf,
  getMeta,
  graphs: new Graphs(readChunks),
  now: Date.now,
};

const handlerDeps = {keyOf, limiter: new RateLimiter()};

type Params<K extends string> = {params: Promise<Record<K, string>>};

export const listGraphs = (req: Request) =>
  serve(req, ctx => v1.listGraphs(ctx, deps), handlerDeps);

export const retrieveGraph = async (
  req: Request,
  {params}: Params<"graph_id">,
) => {
  const {graph_id} = await params;
  return serve(req, ctx => v1.retrieveGraph(ctx, deps, graph_id), handlerDeps);
};

export const listNodes = async (req: Request, {params}: Params<"graph_id">) => {
  const {graph_id} = await params;
  return serve(req, ctx => v1.listNodes(ctx, deps, graph_id), handlerDeps);
};

export const searchNodes = async (
  req: Request,
  {params}: Params<"graph_id">,
) => {
  const {graph_id} = await params;
  return serve(req, ctx => v1.searchNodes(ctx, deps, graph_id), handlerDeps);
};

export const retrieveNode = async (
  req: Request,
  {params}: Params<"graph_id" | "node_id">,
) => {
  const {graph_id, node_id} = await params;
  return serve(
    req,
    ctx => v1.retrieveNode(ctx, deps, graph_id, node_id),
    handlerDeps,
  );
};

export const listEvents = async (
  req: Request,
  {params}: Params<"graph_id">,
) => {
  const {graph_id} = await params;
  return serve(req, ctx => v1.listEvents(ctx, deps, graph_id), handlerDeps);
};

export const retrieveEvent = async (
  req: Request,
  {params}: Params<"graph_id" | "event_id">,
) => {
  const {graph_id, event_id} = await params;
  return serve(
    req,
    ctx => v1.retrieveEvent(ctx, deps, graph_id, event_id),
    handlerDeps,
  );
};

/** Any other path under /api/v1: there's nothing there. */
export const unknown = (req: Request) =>
  serve(
    req,
    async ctx => {
      throw resourceMissing("endpoint", `GET ${ctx.url.pathname}`, "url");
    },
    handlerDeps,
  );

/**
 * Answers a GET for `/api/v1/<path>` without a route: the same endpoints,
 * found the way app/api/v1/… finds them, so another route can call the API
 * in-process (app/showcase/api/ag/…, the showcase's proxy).
 */
export function dispatch(req: Request, path: Array<string>): Promise<Response> {
  const params = <K extends string>(p: Record<K, string>) => ({
    params: Promise.resolve(p),
  });
  const [top, graph_id, kind, id, ...rest] = path;
  if (top === "graphs" && !rest.length) {
    if (graph_id === undefined) {
      return listGraphs(req);
    }
    if (kind === undefined) {
      return retrieveGraph(req, params({graph_id}));
    }
    if (kind === "nodes") {
      if (id === undefined) {
        return listNodes(req, params({graph_id}));
      }
      if (id === "search") {
        return searchNodes(req, params({graph_id}));
      }
      return retrieveNode(req, params({graph_id, node_id: id}));
    }
    if (kind === "events") {
      if (id === undefined) {
        return listEvents(req, params({graph_id}));
      }
      return retrieveEvent(req, params({graph_id, event_id: id}));
    }
  }
  return unknown(req);
}
