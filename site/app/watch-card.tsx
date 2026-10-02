"use client";

import {useEffect, useRef, useState, useSyncExternalStore} from "react";
import {flushSync} from "react-dom";

import {type Watching, watchingText} from "@/lib/watching";

import {CopyCommand} from "./copy-command";

interface Props {
  loggedOutContent: React.ReactNode;
}

/**
 * For a logged-in visitor, above the install card: whether their
 * `agent-graph watch-remote` is running now, and if so how many sessions
 * it's watching are active and how many completed, from which coding
 * agents, and where it's running (the whole card a link to /watch:
 * lib/watching.ts's `watchingText`); if not, the command that
 * starts it. Asked again whenever the page is shown again or focused, so
 * it's never out of date.
 *
 * It's shown, and changed, as a view transition: the page's cards are
 * named (app/site.css, "view transitions"), so it comes in as the cards
 * below it move down to make room, rather than jumping in.
 */
export function WatchCard(props: Props) {
  const signedIn = useSyncExternalStore(
    () => () => {},
    () => document.cookie.split("; ").includes("ag_signed_in=1"),
    () => false,
  );
  const [state, setState] = useState<Watching | null>(null);
  const shown = useRef<string | null>(null);

  useEffect(() => {
    if (!signedIn) {
      return;
    }
    // Shown, moving what's below, only when it changes.
    const show = (next: Watching) => {
      const key = JSON.stringify(next);
      if (key === shown.current) {
        return;
      }
      shown.current = key;
      const update = () => flushSync(() => setState(next));
      const still = matchMedia("(prefers-reduced-motion: reduce)").matches;
      if (document.startViewTransition && !still) {
        // (Skipped, it still updates: in a hidden tab, say.)
        document.startViewTransition(update).ready.catch(() => {});
      } else {
        update();
      }
    };
    let asking: AbortController | null = null;
    const ask = () => {
      if (document.visibilityState !== "visible") {
        return;
      }
      asking?.abort();
      const now = new AbortController();
      asking = now;
      fetch("/api/watching", {cache: "no-store", signal: now.signal})
        .then(res => (res.ok ? (res.json() as Promise<Watching>) : null))
        .then(next => {
          if (next && !now.signal.aborted) {
            show(next);
          }
        })
        // Offline, say: what's shown stays till it can be asked again.
        .catch(() => {});
    };
    ask();
    document.addEventListener("visibilitychange", ask);
    window.addEventListener("focus", ask);
    // (Back to the page from another, from the browser's cache.)
    window.addEventListener("pageshow", ask);
    return () => {
      asking?.abort();
      document.removeEventListener("visibilitychange", ask);
      window.removeEventListener("focus", ask);
      window.removeEventListener("pageshow", ask);
    };
  }, [signedIn]);

  if (!signedIn || !state) {
    return props.loggedOutContent;
  }
  return state.watching ? (
    <a className="card watch-card live" href="/watch">
      <span className="watch-dot" aria-hidden="true" />
      <WatchingText {...state} />
      <span className="watch-go" aria-hidden="true">
        Open →
      </span>
    </a>
  ) : signedIn ? (
    <div className="card watch-card">
      <div className="watch-text">
        <strong>Not watching anything right now</strong>
        <span className="watch-sub">
          To watch your sessions live from here, run this in your terminal:
        </span>
        <CopyCommand command="agent-graph watch-remote" />
      </div>
    </div>
  ) : (
    props.loggedOutContent
  );
}

function WatchingText(props: Extract<Watching, {watching: true}>) {
  const {title, lines} = watchingText(props);
  return (
    <span className="watch-text">
      <strong>{title}</strong>
      {lines.map(line => (
        <span key={line} className="watch-sub">
          {line}
        </span>
      ))}
    </span>
  );
}
