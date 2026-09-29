"use client";

import {useSyncExternalStore} from "react";

/**
 * "Log in", or "Account" once logged in, for the header, with "Admin"
 * before it for an admin. Whether the browser is logged in, and as an
 * admin, comes from cookies the pages' scripts can read (lib/auth.ts), so
 * the pages themselves stay the same for everyone. (Only the link: /admin
 * checks the session itself.)
 */
export function AccountLink() {
  const signedIn = useSyncExternalStore(
    () => () => {},
    () => document.cookie.split("; ").includes("ag_signed_in=1"),
    () => null,
  );
  const admin = useSyncExternalStore(
    () => () => {},
    () => document.cookie.split("; ").includes("ag_admin=1"),
    () => false,
  );
  const next = useSyncExternalStore(
    () => () => {},
    () => location.pathname + location.search,
    () => "/",
  );
  // Nothing until it's known, rather than the wrong one.
  if (signedIn === null) {
    return <span className="account-link" aria-hidden="true" />;
  }
  return signedIn ? (
    <>
      {admin ? (
        <a className="account-link" href="/admin">
          Admin
        </a>
      ) : null}
      <a className="account-link" href="/account">
        Account
      </a>
    </>
  ) : (
    <a className="account-link" href={`/login?${new URLSearchParams({next})}`}>
      Log in
    </a>
  );
}
