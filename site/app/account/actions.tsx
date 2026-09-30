"use client";

import {useRouter} from "next/navigation";
import {useState} from "react";

import type {Computer} from "@/lib/accounts";

import {CopyCommand} from "../copy-command";

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
                Logged in {date(c.createdAt)}, last used{" "}
                {date(c.usedAt ?? c.createdAt)}
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

/**
 * A button that asks the site for one of Stripe's pages (Checkout, or the
 * customer portal: lib/stripe.ts), and goes there.
 */
export function StripeButton({
  path,
  body,
  secondary,
  children,
}: {
  path: string;
  body?: object;
  secondary?: boolean;
  children: React.ReactNode;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  async function go() {
    setBusy(true);
    setError(null);
    try {
      const res = await fetch(path, {
        method: "POST",
        ...(body
          ? {
              headers: {"Content-Type": "application/json"},
              body: JSON.stringify(body),
            }
          : {}),
      });
      if (!res.ok) {
        throw new Error(await res.text());
      }
      location.assign((await res.json()).url);
    } catch (err) {
      setError(
        err instanceof Error && err.message
          ? err.message
          : "That didn't work. Try again.",
      );
      setBusy(false);
    }
  }
  return (
    <>
      <button
        className={secondary ? "button secondary" : "button"}
        type="button"
        onClick={go}
        disabled={busy}>
        {busy ? "Opening Stripe…" : children}
      </button>
      {error ? (
        <span className="error" style={{display: "block"}}>
          {error}
        </span>
      ) : null}
    </>
  );
}

/**
 * Deleting the account, for good (DELETE /api/account): what goes, spelled
 * out, and a button that works only once the account's email address (or
 * "delete", for one without) is typed. If the last sign-in wasn't recent,
 * it asks to log in again first.
 */
export function DeleteAccount({
  email,
  subscribed,
}: {
  email: string | null;
  /** Whether there's a subscription it'll cancel. */
  subscribed: boolean;
}) {
  const [open, setOpen] = useState(false);
  const [typed, setTyped] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [signInAgain, setSignInAgain] = useState(false);
  const expected = email ?? "delete";
  const matches = typed.trim().toLowerCase() === expected.toLowerCase();

  async function remove() {
    setBusy(true);
    setError(null);
    try {
      const res = await fetch("/api/account", {
        method: "DELETE",
        headers: {"Content-Type": "application/json"},
        body: JSON.stringify({confirm: typed}),
      });
      if (res.status === 401) {
        const reply = await res.json().catch(() => null);
        if (reply?.signInAgain) {
          setSignInAgain(true);
          setBusy(false);
          return;
        }
      }
      if (!res.ok) {
        throw new Error(await res.text());
      }
      location.assign("/?deleted=1");
    } catch (err) {
      setError(
        err instanceof Error && err.message
          ? err.message
          : "That didn't work. Try again.",
      );
      setBusy(false);
    }
  }

  if (!open) {
    return (
      <button
        className="button danger-outline"
        type="button"
        onClick={() => setOpen(true)}>
        Delete my account…
      </button>
    );
  }
  return (
    <div className="danger-zone" role="group" aria-labelledby="delete-title">
      <h3 id="delete-title">Delete your account?</h3>
      <p>This can&rsquo;t be undone. Straight away:</p>
      <ul>
        {subscribed ? (
          <li>
            <strong>Your subscription is cancelled</strong>, with no refund for
            the rest of the period you&rsquo;ve paid for.
          </li>
        ) : null}
        <li>
          <strong>Your live shares are deleted</strong>, and /watch shows
          nothing.
        </li>
        <li>
          <strong>Every computer is logged out</strong>:{" "}
          <code>agent-graph watch-remote</code> stops sharing, and has to log in
          again (to a new account).
        </li>
        <li>
          <strong>Your login and account are deleted.</strong> Logging in again
          with the same email makes a new, empty account.
        </li>
      </ul>
      <p className="muted">
        Logs you pasted or uploaded aren&rsquo;t part of your account: anyone
        with their links can still open them until they expire, a week after
        they were last added to. Nothing on your own computer is touched.
      </p>
      {signInAgain ? (
        <p className="error">
          To be sure it&rsquo;s you, log in again first, then come back here:{" "}
          <a href="/login?again=1&next=/account%23delete">log in again</a>.
        </p>
      ) : (
        <>
          <label className="field delete-field">
            <span>
              To confirm, type{" "}
              {email ? "your email address" : <>&ldquo;delete&rdquo;</>}:{" "}
              <strong>{expected}</strong>
            </span>
            <input
              type="text"
              autoComplete="off"
              autoCapitalize="off"
              spellCheck={false}
              value={typed}
              onChange={e => setTyped(e.target.value)}
              disabled={busy}
            />
          </label>
          <div className="danger-actions">
            <button
              className="button danger"
              type="button"
              onClick={remove}
              disabled={!matches || busy}>
              {busy ? "Deleting…" : "Delete my account for good"}
            </button>
            <button
              className="button secondary"
              type="button"
              onClick={() => {
                setOpen(false);
                setTyped("");
                setError(null);
              }}
              disabled={busy}>
              Keep my account
            </button>
          </div>
        </>
      )}
      {error ? <p className="error">{error}</p> : null}
    </div>
  );
}

/**
 * API tokens (lib/accounts.ts): for a machine that can't log in in a
 * browser, such as a cloud instance. Each is named, shown once when it's
 * made, to copy, and can be revoked; `agent-graph watch-remote` uses one
 * given as AGENT_GRAPH_TOKEN.
 */
export function ApiTokens({tokens}: {tokens: Array<Computer>}) {
  const router = useRouter();
  const [name, setName] = useState("");
  const [made, setMade] = useState<{name: string; token: string} | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function create(event: React.FormEvent) {
    event.preventDefault();
    setBusy("new");
    setError(null);
    try {
      const res = await fetch("/api/account/tokens", {
        method: "POST",
        headers: {"Content-Type": "application/json"},
        body: JSON.stringify({name}),
      });
      if (!res.ok) {
        throw new Error(await res.text());
      }
      const {token} = (await res.json()) as {token: string};
      setMade({name: name.trim(), token});
      setName("");
      router.refresh();
    } catch (err) {
      setError(
        err instanceof Error && err.message
          ? err.message
          : "That didn't work. Try again.",
      );
    }
    setBusy(null);
  }

  async function revoke(id: string) {
    setBusy(id);
    setError(null);
    const res = await fetch(`/api/account/computers/${id}`, {method: "DELETE"});
    if (!res.ok) {
      setError(await res.text());
    }
    setBusy(null);
    router.refresh();
  }

  const date = (ms: number) =>
    new Date(ms).toLocaleDateString("en-GB", {
      day: "numeric",
      month: "short",
      year: "numeric",
      timeZone: "UTC",
    });
  return (
    <>
      <p>
        For a machine where you can&rsquo;t log in in a browser, such as a cloud
        instance: make a token here, and give it to{" "}
        <code>agent-graph watch-remote</code> there as{" "}
        <code>AGENT_GRAPH_TOKEN</code>. It shares to this account, like a
        computer you&rsquo;ve logged in on. Anyone who has it can too, so keep
        it secret, and revoke it when you&rsquo;re done with it.
      </p>
      {made ? (
        <div className="token-made" role="status">
          <p>
            <strong>Your token for &ldquo;{made.name}&rdquo;.</strong> Copy it
            now: it won&rsquo;t be shown again.
          </p>
          <CopyCommand command={`export AGENT_GRAPH_TOKEN=${made.token}`} />
          <p className="muted">
            Then, on that machine, <code>agent-graph watch-remote</code> (or{" "}
            <code>--autostart</code>) shares to your account, without logging
            in.
          </p>
          <button
            className="link-button"
            type="button"
            onClick={() => setMade(null)}>
            Done
          </button>
        </div>
      ) : null}
      {tokens.length ? (
        <ul className="computers">
          {tokens.map(t => (
            <li key={t.id}>
              <span>
                <strong>{t.host || "An API token"}</strong>
                <br />
                <span className="when">
                  Made {date(t.createdAt)},{" "}
                  {t.usedAt ? `last used ${date(t.usedAt)}` : "not used yet"}
                </span>
              </span>
              <button
                className="link-button"
                type="button"
                onClick={() => revoke(t.id)}
                disabled={busy === t.id}>
                {busy === t.id ? "Revoking…" : "Revoke"}
              </button>
            </li>
          ))}
        </ul>
      ) : null}
      <form className="token-form" onSubmit={create}>
        <label className="field">
          <span>Name it for where it&rsquo;s used</span>
          <input
            type="text"
            placeholder="e.g. build server"
            maxLength={100}
            value={name}
            onChange={e => setName(e.target.value)}
            disabled={busy === "new"}
          />
        </label>
        <button
          className="button secondary"
          type="submit"
          disabled={!name.trim() || busy === "new"}>
          {busy === "new" ? "Making…" : "Make a token"}
        </button>
      </form>
      {error ? <p className="error">{error}</p> : null}
    </>
  );
}
