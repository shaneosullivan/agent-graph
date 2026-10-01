/**
 * What the site counts, and who can see the counts: the parts of
 * lib/analytics.ts with no database, and who /admin is for (`isAdmin`).
 *
 * Everything counted is a counter, by name, per day and per month (UTC):
 *
 *   pageview                 a page shown (the browser says so: app/analytics.tsx)
 *   visitor                  a browser's first page that day, or month (see below)
 *   download:<target>        a download button clicked (app/install.tsx), per target
 *   download:copy:<command>  an install command copied (its copy button), per command
 *   watch                    a live share started, or carried on with (`agent-graph watch-remote`)
 *   watch.new                of those, a new share
 *   signup                   an account made (its first login)
 *
 * Visitors are counted without a cookie: the browser keeps the day and the
 * month it last said it was a visitor (localStorage), and says so again only
 * in a new one. So a browser's a visitor once a day, and once a month.
 *
 * No imports: the unit tests load it directly.
 */

/** The programs a release builds: dist-workspace.toml's targets. */
export const TARGETS = [
  "aarch64-apple-darwin",
  "x86_64-apple-darwin",
  "aarch64-unknown-linux-musl",
  "x86_64-unknown-linux-musl",
  "aarch64-pc-windows-msvc",
  "x86_64-pc-windows-msvc",
] as const;
export type Target = (typeof TARGETS)[number];

/**
 * The commands the install section shows, with a button to copy each
 * (app/copy-command.tsx): a copy counts as a download of its own kind.
 */
export const COPIED = [
  "homebrew",
  "install-script",
  "claude-code",
  "claude-code-cloud",
  "codex",
  "codex-cloud",
] as const;
export type Copied = (typeof COPIED)[number];

/** Every counter there can be. */
export const COUNTERS = [
  "pageview",
  "visitor",
  "watch",
  "watch.new",
  "signup",
  ...TARGETS.map(t => `download:${t}` as const),
  ...COPIED.map(c => `download:copy:${c}` as const),
] as const;
export type Counter = (typeof COUNTERS)[number];

/** Whether nothing's to be counted: ANALYTICS_DISABLED is "true". */
export function analyticsDisabled(
  env: Record<string, string | undefined> = process.env,
): boolean {
  return env.ANALYTICS_DISABLED?.trim().toLowerCase() === "true";
}

/** The day (2026-09-29) and month (2026-09) of `at`, in UTC. */
export function periods(at: Date): {day: string; month: string} {
  const day = at.toISOString().slice(0, 10);
  return {day, month: day.slice(0, 7)};
}

/**
 * What a browser's report counts (POST /api/analytics), if it's a report:
 *
 *   {event: "pageview", newDay?: true, newMonth?: true}
 *   {event: "download", target: <one of TARGETS>}
 *   {event: "download", copied: <one of COPIED>}
 *
 * A page view counts the visitor too, for the day and the month it's their
 * first in. Anything else counts nothing.
 */
export function countsOf(
  report: unknown,
): {day: Array<Counter>; month: Array<Counter>} | null {
  if (!report || typeof report !== "object") {
    return null;
  }
  const r = report as Record<string, unknown>;
  if (r.event === "pageview") {
    return {
      day: r.newDay === true ? ["pageview", "visitor"] : ["pageview"],
      month: r.newMonth === true ? ["pageview", "visitor"] : ["pageview"],
    };
  }
  if (r.event === "download" && TARGETS.includes(r.target as Target)) {
    const counter = `download:${r.target as Target}` as const;
    return {day: [counter], month: [counter]};
  }
  if (r.event === "download" && COPIED.includes(r.copied as Copied)) {
    const counter = `download:copy:${r.copied as Copied}` as const;
    return {day: [counter], month: [counter]};
  }
  return null;
}

/**
 * The addresses in ADMIN_EMAILS: comma-separated, any case, spaces around
 * them ignored.
 */
export function adminEmails(
  env: Record<string, string | undefined> = process.env,
): Set<string> {
  return new Set(
    (env.ADMIN_EMAILS ?? "")
      .split(",")
      .map(e => e.trim().toLowerCase())
      .filter(Boolean),
  );
}

/**
 * Whether an account can see /admin: its email's in ADMIN_EMAILS, and
 * verified (Google's are). An unverified one could be anyone's: an account
 * made with an email and password, say, before its owner signed up.
 */
export function isAdmin(
  user: {email: string | null; emailVerified: boolean} | null,
  env: Record<string, string | undefined> = process.env,
): boolean {
  return Boolean(
    user?.email &&
    user.emailVerified &&
    adminEmails(env).has(user.email.toLowerCase()),
  );
}

/** How long a day's counts are kept (the month's are kept for good). */
export const DAYS_KEPT = 365;

/** The first day kept at `now`: days before it are deleted (lib/analytics.ts). */
export function firstDayKept(now: Date): string {
  return periods(new Date(now.getTime() - DAYS_KEPT * 24 * 60 * 60 * 1000)).day;
}
