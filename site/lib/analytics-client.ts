/**
 * Reporting what a browser does, for /admin (what's counted:
 * lib/analytics-core.ts), to POST /api/analytics. A report's sent as the
 * page is left, if it's being left (`sendBeacon`), and never gets in the
 * way: one that can't be sent is dropped.
 *
 * The page renders <Analytics /> (app/analytics.tsx) only when counting's
 * on, and `track` does nothing until it has.
 */

let enabled = false;

/** Turns reporting on: app/analytics.tsx, when ANALYTICS_DISABLED isn't true. */
export function enableAnalytics(): void {
  enabled = true;
}

export type Report =
  | {event: "pageview"; newDay?: true; newMonth?: true}
  | {event: "download"; target: string}
  | {event: "download"; copied: string};

/** Reports `report`, if reporting's on. */
export function track(report: Report): void {
  if (!enabled) {
    return;
  }
  const body = JSON.stringify(report);
  try {
    if (
      navigator.sendBeacon?.(
        "/api/analytics",
        new Blob([body], {type: "application/json"}),
      )
    ) {
      return;
    }
  } catch {
    // Sent the other way, below.
  }
  fetch("/api/analytics", {
    method: "POST",
    headers: {"Content-Type": "application/json"},
    body,
    keepalive: true,
  }).catch(() => {});
}

/**
 * Whether this browser's not been counted as a visitor on `key`'s period
 * yet (today, or this month), and remembers that it now has. It keeps only
 * the period, no id: nothing that tells browsers apart.
 */
function firstIn(key: string, period: string): boolean {
  try {
    if (localStorage.getItem(key) === period) {
      return false;
    }
    localStorage.setItem(key, period);
    return true;
  } catch {
    // Without storage, every page would count a visitor: count none.
    return false;
  }
}

/** Reports a page shown, and whether it's this browser's first today, or this month. */
export function trackPageview(): void {
  const day = new Date().toISOString().slice(0, 10);
  const newDay = firstIn("agentGraphVisitedDay", day);
  const newMonth = firstIn("agentGraphVisitedMonth", day.slice(0, 7));
  track({
    event: "pageview",
    ...(newDay ? {newDay: true as const} : {}),
    ...(newMonth ? {newMonth: true as const} : {}),
  });
}
