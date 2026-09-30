"use client";

import {useSyncExternalStore} from "react";

/**
 * Says the account's been deleted, on the home page it's sent to after
 * (`?deleted=1`: app/account/actions.tsx's DeleteAccount). Nothing
 * otherwise.
 */
export function DeletedNotice() {
  const deleted = useSyncExternalStore(
    () => () => {},
    () => new URLSearchParams(location.search).get("deleted") === "1",
    () => false,
  );
  if (!deleted) {
    return null;
  }
  return (
    <p className="card deleted-notice" role="status">
      Your account has been deleted, with its live shares, its computers&rsquo;
      logins and any subscription. Thanks for trying Agent Graph.
    </p>
  );
}
