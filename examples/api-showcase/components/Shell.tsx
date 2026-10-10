"use client";

import Link from "next/link";
import {usePathname} from "next/navigation";
import {Suspense, useEffect} from "react";

import {useCallLog} from "@/lib/client";
import {DEMO, SOURCE} from "@/lib/demo";
import {graphHref, useGraph} from "@/lib/graph";
import {UnderTheHood} from "./UnderTheHood";

export const SECTIONS = [
  {slug: "", label: "Overview", icon: "◎", blurb: "Where everything stands"},
  {
    slug: "explorer",
    label: "Tree explorer",
    icon: "⑂",
    blurb: "Every agent, and what it started",
  },
  {
    slug: "time-machine",
    label: "Time machine",
    icon: "⟲",
    blurb: "Replay the graph, event by event",
  },
  {
    slug: "bottlenecks",
    label: "Bottlenecks",
    icon: "⧗",
    blurb: "What's stuck, and what it's holding up",
  },
  {
    slug: "performance",
    label: "Performance",
    icon: "⏱",
    blurb: "Where the time goes",
  },
  {
    slug: "live",
    label: "Live feed",
    icon: "●",
    blurb: "Follow it as it happens",
  },
  {
    slug: "search",
    label: "Search",
    icon: "⌕",
    blurb: "Ask the graph a question",
  },
  {
    slug: "briefing",
    label: "AI briefing",
    icon: "✦",
    blurb: "A snapshot for another model",
  },
];

export function Shell({
  apiUrl,
  children,
}: {
  apiUrl: string;
  children: React.ReactNode;
}) {
  const path = usePathname();
  const {clear} = useCallLog();
  // Each page shows its own calls.
  useEffect(() => clear(), [path, clear]);

  return (
    <div className="shell">
      <aside className="sidebar">
        <Link href="/" className="brand">
          <span className="brand-mark" aria-hidden>
            <svg
              width="18"
              height="18"
              viewBox="0 0 24 24"
              fill="none"
              stroke="white"
              strokeWidth="2.2">
              <circle cx="6" cy="6" r="2.6" />
              <circle cx="18" cy="8" r="2.6" />
              <circle cx="9" cy="18" r="2.6" />
              <path d="M8.3 7.3 15.6 7.7M7 8.5l1.4 7M11.3 16.6l5.2-6.4" />
            </svg>
          </span>
          <span>
            Agent Graph API
            <small>A showcase</small>
          </span>
        </Link>

        <nav className="nav-group">
          <h4>Start</h4>
          <Link href="/" className={`nav-link ${path === "/" ? "active" : ""}`}>
            <span className="icon">⌂</span> Your graphs
          </Link>
        </nav>

        <Suspense fallback={<GraphNav graph={null} path={path} />}>
          <GraphNavHere path={path} />
        </Suspense>

        <div className="spacer" />
        <p className="faint" style={{fontSize: 12, margin: 0}}>
          {DEMO ? (
            <>
              This is a demo account&rsquo;s graph, as it stood at its last event.{" "}
              <a href={SOURCE} target="_blank" rel="noreferrer">
                Run this app on your own graphs ↗
              </a>{" "}
            </>
          ) : (
            <>Every view calls the API live, with your key, through this app&rsquo;s server. </>
          )}
          Open <b>Under the hood</b> at the foot of any page to see the calls.{" "}
          <a
            href="https://agentgraph.chofter.com/docs/reference"
            target="_blank"
            rel="noreferrer">
            API reference ↗
          </a>
        </p>
      </aside>
      <main className="main">
        {children}
        <UnderTheHood apiUrl={apiUrl} />
      </main>
    </div>
  );
}

function GraphNavHere({path}: {path: string}) {
  return <GraphNav graph={useGraph()} path={path} />;
}

/** Each view of the graph the address names (none to go to until it names one). */
function GraphNav({graph, path}: {graph: string | null; path: string}) {
  return (
    <>
      <nav className="nav-group">
        <h4>{graph ? "This graph" : "Pick a graph first"}</h4>
        {SECTIONS.map(s => {
          const view = `/g${s.slug ? `/${s.slug}` : ""}`;
          return (
            <Link
              key={s.slug}
              href={graph ? graphHref(graph, s.slug) : "#"}
              className={`nav-link ${graph && path === view ? "active" : ""} ${graph ? "" : "disabled"}`}>
              <span className="icon">{s.icon}</span> {s.label}
            </Link>
          );
        })}
      </nav>
      {graph ? (
        <div className="graph-pill">
          Reading
          <span className="mono">{graph}</span>
        </div>
      ) : null}
    </>
  );
}
