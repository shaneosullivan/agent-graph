"use client";

// Which graph a view is of: `?graph=gph_…` in its address.

import Link from "next/link";
import {usePathname, useRouter, useSearchParams} from "next/navigation";
import {Suspense, useEffect, useState} from "react";

import {useApi} from "./client";
import type {Graph, List} from "./types";

/** The graph the address names, if it names one. */
export function useGraph(): string | null {
  return useSearchParams().get("graph");
}

/** A view's address: `/showcase/g/<view>?graph=…`, and anything else it takes. */
export function graphHref(
  graph: string,
  view = "",
  extra: Record<string, string> = {},
): string {
  const query = new URLSearchParams({graph, ...extra});
  return `/showcase/g${view ? `/${view}` : ""}?${query}`;
}

/** A view of one graph: rendered once the address says which. */
export function GraphPage({
  children,
}: {
  children: (graph: string) => React.ReactNode;
}) {
  return (
    <Suspense fallback={null}>
      <WithGraph render={children} />
    </Suspense>
  );
}

function WithGraph({render}: {render: (graph: string) => React.ReactNode}) {
  const graph = useGraph();
  if (!graph) {
    return <FirstGraph />;
  }
  return <>{render(graph)}</>;
}

/**
 * A view opened without a graph (from the menu, before one's been picked):
 * the same view, of the first graph listed.
 */
function FirstGraph() {
  const {get} = useApi();
  const router = useRouter();
  const view = usePathname().replace(/^\/showcase\/g\/?/, "");
  const [none, setNone] = useState(false);
  useEffect(() => {
    let live = true;
    get<List<Graph>>("/graphs", {limit: 1}).then(
      ({data}) => {
        const first = data?.data[0]?.id;
        if (!live) {
          return;
        }
        if (first) {
          router.replace(graphHref(first, view));
        } else {
          setNone(true);
        }
      },
      () => live && setNone(true),
    );
    return () => {
      live = false;
    };
  }, [get, router, view]);
  return none ? (
    <div className="empty">
      There&rsquo;s no graph to show yet. <Link href="/showcase">Back</Link>.
    </div>
  ) : null;
}
