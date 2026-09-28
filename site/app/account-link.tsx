"use client";

import { useSyncExternalStore } from "react";

/**
 * "Log in", or "Account" once logged in, for the header. Whether the browser
 * is logged in comes from a cookie the pages' scripts can read
 * (lib/auth.ts), so the pages themselves stay the same for everyone.
 */
export function AccountLink() {
  const signedIn = useSyncExternalStore(
    () => () => {},
    () => document.cookie.split("; ").includes("ag_signed_in=1"),
    () => null,
  );
  const next = useSyncExternalStore(
    () => () => {},
    () => location.pathname + location.search,
    () => "/",
  );
  // Nothing until it's known, rather than the wrong one.
  if (signedIn === null) return <span className="account-link" aria-hidden="true" />;
  return signedIn ? (
    <a className="account-link" href="/account">
      Account
    </a>
  ) : (
    <a className="account-link" href={`/login?${new URLSearchParams({ next })}`}>
      Log in
    </a>
  );
}
