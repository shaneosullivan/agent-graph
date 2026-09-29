/**
 * Which version of the site a page is, and whether it should reload to
 * the one that's live (app/app-updates.tsx). No imports: the unit tests
 * load it directly.
 *
 * Each build has an id (scripts/write-version.mjs: Vercel's deployment id,
 * or the commit, or the time locally), written to public/version.json and
 * compiled into its pages, so a page can fetch the live build's and
 * compare.
 */

/** This page's build. */
export const BUILD_ID = process.env.NEXT_PUBLIC_BUILD_ID ?? "";

/**
 * Whether a page of build `current` should reload now that `live` is
 * the site's build:
 *
 * - not when either isn't known, or they're the same;
 * - not twice for the same live build (`reloadedFor`, kept for the tab),
 *   so a page that still comes back old (a cache on the way, say) can't
 *   reload over and over;
 * - not while something's typed in the page (`typing`), which a reload
 *   would lose: a later check does it once that's gone.
 */
export function shouldReload({
  current,
  live,
  reloadedFor,
  typing,
}: {
  current: string;
  live: string | null;
  reloadedFor: string | null;
  typing: boolean;
}): boolean {
  return Boolean(
    current && live && live !== current && reloadedFor !== live && !typing,
  );
}
