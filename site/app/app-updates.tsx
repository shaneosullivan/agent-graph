"use client";

import {usePathname} from "next/navigation";
import {useEffect} from "react";

import {BUILD_ID, shouldReload} from "@/lib/app-version";

/** Checks closer together than this are skipped (focus comes in bursts). */
const MIN_GAP_MS = 5000;
/** The tab's record of the build it last reloaded for (see shouldReload). */
const RELOADED_FOR = "agentGraphReloadedFor";

/**
 * Keeps an open page, or the installed app, on the site's live version:
 * whenever it's loaded, focused, shown again or moved to another page, it
 * fetches the live build's id (/version.json), and reloads if it isn't
 * this page's. A script of the site's that fails to load (an old page
 * asking for a file a new build no longer has) checks at once.
 *
 * It also registers the service worker (public/sw.js), which makes the
 * site installable and caches nothing, so it can't keep anything stale.
 */
export function AppUpdates() {
  const path = usePathname();

  useEffect(() => {
    if ("serviceWorker" in navigator) {
      // Always fetched fresh, not from the HTTP cache, so a new one's seen.
      navigator.serviceWorker
        .register("/sw.js", {scope: "/", updateViaCache: "none"})
        .catch(() => {});
    }
    const onFocus = () => void check();
    const onVisible = () => {
      if (document.visibilityState === "visible") {
        void check();
      }
    };
    const onError = (event: Event) => {
      const el = event.target as HTMLElement | null;
      const url =
        el instanceof HTMLScriptElement
          ? el.src
          : el instanceof HTMLLinkElement
            ? el.href
            : "";
      if (url.includes("/_next/")) {
        void check(true);
      }
    };
    window.addEventListener("focus", onFocus);
    document.addEventListener("visibilitychange", onVisible);
    // Captured: a script's or stylesheet's error doesn't bubble.
    window.addEventListener("error", onError, true);
    return () => {
      window.removeEventListener("focus", onFocus);
      document.removeEventListener("visibilitychange", onVisible);
      window.removeEventListener("error", onError, true);
    };
  }, []);

  // On load, and on each move to another page within the site.
  useEffect(() => {
    void check();
  }, [path]);

  return null;
}

let lastCheck = 0;

/** Asks which build is live, and reloads if it's not this page's. */
async function check(now = false): Promise<void> {
  if (!BUILD_ID || (!now && Date.now() - lastCheck < MIN_GAP_MS)) {
    return;
  }
  lastCheck = Date.now();
  void navigator.serviceWorker
    ?.getRegistration()
    .then(r => r?.update())
    .catch(() => {});
  let live: string | null = null;
  try {
    // A new URL each time, so nothing on the way can answer from a cache.
    const res = await fetch(`/version.json?t=${Date.now()}`, {
      cache: "no-store",
    });
    live = res.ok
      ? (((await res.json()) as {version?: string}).version ?? null)
      : null;
  } catch {
    // Offline, or the site's down: nothing to compare with.
    return;
  }
  const reload = shouldReload({
    current: BUILD_ID,
    live,
    reloadedFor: read(RELOADED_FOR),
    typing: typing(),
  });
  if (reload && live) {
    write(RELOADED_FOR, live);
    location.reload();
  }
}

/** Fields whose contents a reload would lose: text of any kind, or a chosen file. */
const TYPED = new Set([
  "text",
  "email",
  "password",
  "search",
  "url",
  "tel",
  "number",
  "file",
]);

/**
 * The fields someone has typed into (or chosen a file in). Tracked as it
 * happens: a React field's `defaultValue` follows its value, so the value
 * alone can't tell what's been typed.
 */
const edited = new Set<HTMLInputElement | HTMLTextAreaElement>();

if (typeof document !== "undefined") {
  const onEdit = (event: Event) => {
    const el = event.target;
    if (
      el instanceof HTMLTextAreaElement ||
      (el instanceof HTMLInputElement && TYPED.has(el.type))
    ) {
      edited.add(el);
    }
  };
  document.addEventListener("input", onEdit, true);
  document.addEventListener("change", onEdit, true);
}

/**
 * Whether something's typed in the page (or a file chosen), which a reload
 * would lose: a field that's been typed into and isn't empty now. A slider
 * or a checkbox moved doesn't count.
 */
function typing(): boolean {
  for (const el of edited) {
    if (!el.isConnected) {
      edited.delete(el);
    } else if (el.value !== "") {
      return true;
    }
  }
  return false;
}

function read(key: string): string | null {
  try {
    return sessionStorage.getItem(key);
  } catch {
    return null;
  }
}

function write(key: string, value: string): void {
  try {
    sessionStorage.setItem(key, value);
  } catch {
    // Without storage, only a page that came back stale after reloading
    // could reload again, and pages aren't cached (max-age=0, and the
    // service worker caches nothing).
  }
}
