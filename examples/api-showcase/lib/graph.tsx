"use client";

// Which graph a view is of: `?graph=gph_…` in its address (not a path
// segment, so the app can also be built as static pages: see next.config.ts).

import Link from "next/link";
import {useSearchParams} from "next/navigation";
import {Suspense} from "react";

/** The graph the address names, if it names one. */
export function useGraph(): string | null {
  return useSearchParams().get("graph");
}

/** A view's address: `/g/<view>?graph=…`, and anything else it takes. */
export function graphHref(graph: string, view = "", extra: Record<string, string> = {}): string {
  const query = new URLSearchParams({graph, ...extra});
  return `/g${view ? `/${view}` : ""}?${query}`;
}

/** A view of one graph: rendered once the address says which. */
export function GraphPage({children}: {children: (graph: string) => React.ReactNode}) {
  return (
    <Suspense fallback={null}>
      <WithGraph render={children} />
    </Suspense>
  );
}

function WithGraph({render}: {render: (graph: string) => React.ReactNode}) {
  const graph = useGraph();
  if (!graph) {
    return (
      <div className="empty">
        No graph chosen. <Link href="/">Pick one</Link>.
      </div>
    );
  }
  return <>{render(graph)}</>;
}
