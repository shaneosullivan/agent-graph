"use client";

import Link from "next/link";
import {usePathname, useRouter} from "next/navigation";
import {Fragment, Suspense, useEffect, useState} from "react";

import {useCallLog} from "@/app/showcase/_lib/client";
import {graphHref, useGraph} from "@/app/showcase/_lib/graph";
import {type Source, useSource} from "@/app/showcase/_lib/source";
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
  const {source} = useSource();
  // Each page shows its own calls, and each source.
  useEffect(() => clear(), [path, source, clear]);

  // On a phone, the sidebar's a menu (showcase.css): it closes when a link
  // in it is followed, the scrim's tapped, or Escape's pressed.
  const [menu, setMenu] = useState(false);
  useEffect(() => {
    if (!menu) {
      return;
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setMenu(false);
      }
    };
    addEventListener("keydown", onKey);
    return () => removeEventListener("keydown", onKey);
  }, [menu]);
  const here =
    SECTIONS.find(s => path === `/showcase/g${s.slug ? `/${s.slug}` : ""}`)
      ?.label ?? (source === "demo" ? "The demo\u2019s graphs" : "Your graphs");

  return (
    <div className="shell">
      <header className="topbar">
        <Link href="/showcase" aria-label="The showcase's home">
          <BrandMark />
        </Link>
        <span className="topbar-title">
          <b>{here}</b>
          <small>Agent Graph API showcase</small>
        </span>
        <button
          type="button"
          className="menu-open"
          aria-label="Menu"
          aria-expanded={menu}
          aria-controls="showcase-menu"
          onClick={() => setMenu(true)}>
          <svg viewBox="0 0 24 24" aria-hidden>
            <path d="M4 7h16M4 12h16M4 17h16" />
          </svg>
        </button>
      </header>
      <div
        className={`menu-scrim ${menu ? "open" : ""}`}
        aria-hidden
        onClick={() => setMenu(false)}
      />
      <aside
        id="showcase-menu"
        className={`sidebar ${menu ? "open" : ""}`}
        onClick={e => {
          if ((e.target as Element).closest("a")) {
            setMenu(false);
          }
        }}>
        <div className="sidebar-head">
          <Link href="/showcase" className="brand">
            <BrandMark />
            <span>
              Agent Graph API
              <small>A showcase</small>
            </span>
          </Link>
          <button
            type="button"
            className="menu-close"
            aria-label="Close menu"
            onClick={() => setMenu(false)}>
            <svg viewBox="0 0 24 24" aria-hidden>
              <path d="M6 6l12 12M18 6L6 18" />
            </svg>
          </button>
        </div>

        <nav className="nav-group">
          <h4>Start</h4>
          <Link
            href="/showcase"
            className={`nav-link ${path === "/showcase" ? "active" : ""}`}>
            <span className="icon">⌂</span>{" "}
            {source === "demo" ? "The demo\u2019s graphs" : "Your graphs"}
          </Link>
        </nav>

        <Suspense fallback={<GraphNav graph={null} path={path} />}>
          <GraphNavHere path={path} />
        </Suspense>

        <div className="spacer" />
        <p className="faint" style={{fontSize: 12, margin: 0}}>
          {source === "demo" ? (
            <>
              This is a demo account&rsquo;s graphs, as they stood at their last
              events. Read your own the same way with a key from your{" "}
              <a href="/account#api-keys">account page</a>.{" "}
            </>
          ) : (
            <>
              These are your account&rsquo;s graphs, read live as you&rsquo;re
              logged in.{" "}
            </>
          )}
          Open <b>Under the hood</b> at the foot of any page to see the calls.{" "}
          <a href="/docs/reference">API reference</a> ·{" "}
          <a href="/">Agent Graph</a>
        </p>
      </aside>
      <main className="main">
        <SourceToggle />
        {/* A new source starts every view afresh. */}
        <Fragment key={source}>{children}</Fragment>
        <UnderTheHood apiUrl={apiUrl} />
      </main>
    </div>
  );
}

function BrandMark() {
  return (
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
  );
}

/**
 * Whose graphs to read, at the top right, for a browser that's logged in:
 * the demo's (always to begin with) or its own. Switching goes back to the
 * list of graphs: a graph of one isn't the other's.
 */
function SourceToggle() {
  const {source, setSource, signedIn} = useSource();
  const router = useRouter();
  if (!signedIn) {
    return null;
  }
  const pick = (next: Source) => {
    if (next === source) {
      return;
    }
    setSource(next);
    router.push("/showcase");
  };
  return (
    <div className="source-toggle" role="radiogroup" aria-label="Whose data">
      {(
        [
          ["demo", "Demo data"],
          ["mine", "Your data"],
        ] as const
      ).map(([value, label]) => (
        <button
          key={value}
          type="button"
          role="radio"
          aria-checked={source === value}
          className={source === value ? "on" : undefined}
          onClick={() => pick(value)}>
          {label}
        </button>
      ))}
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
          const view = `/showcase/g${s.slug ? `/${s.slug}` : ""}`;
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
