"use client";

import {useRouter} from "next/navigation";
import {useState} from "react";

import type {Computer} from "@/lib/accounts";

/** The computers agent-graph is logged in on, each with a way to log it out. */
export function Computers({computers}: {computers: Array<Computer>}) {
  const router = useRouter();
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function remove(id: string) {
    setBusy(id);
    setError(null);
    const res = await fetch(`/api/account/computers/${id}`, {method: "DELETE"});
    if (!res.ok) {
      setError(await res.text());
    }
    setBusy(null);
    router.refresh();
  }

  if (!computers.length) {
    return (
      <p>
        agent-graph isn&rsquo;t logged in on any computer.{" "}
        <code>agent-graph watch-remote</code> asks you to log in the first time.
      </p>
    );
  }
  // The same on the server and in the browser, whatever either's locale.
  const date = (ms: number) =>
    new Date(ms).toLocaleDateString("en-GB", {
      day: "numeric",
      month: "short",
      year: "numeric",
      timeZone: "UTC",
    });
  return (
    <>
      <p>agent-graph is logged in on these. Removing one logs it out there.</p>
      <ul className="computers">
        {computers.map(c => (
          <li key={c.id}>
            <span>
              <strong>{c.host || "A computer"}</strong>
              <br />
              <span className="when">
                Logged in {date(c.createdAt)}, last used {date(c.usedAt)}
              </span>
            </span>
            <button
              className="link-button"
              type="button"
              onClick={() => remove(c.id)}
              disabled={busy === c.id}>
              {busy === c.id ? "Removing…" : "Remove"}
            </button>
          </li>
        ))}
      </ul>
      {error ? <p className="error">{error}</p> : null}
    </>
  );
}

export function LogOut() {
  const [busy, setBusy] = useState(false);
  async function logOut() {
    setBusy(true);
    await fetch("/api/session", {method: "DELETE"}).catch(() => {});
    location.assign("/");
  }
  return (
    <button
      className="button secondary"
      type="button"
      onClick={logOut}
      disabled={busy}
      style={{marginTop: 24}}>
      {busy ? "Logging out…" : "Log out of this browser"}
    </button>
  );
}
