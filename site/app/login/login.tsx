"use client";

import {
  createUserWithEmailAndPassword,
  getRedirectResult,
  GoogleAuthProvider,
  sendPasswordResetEmail,
  signInWithEmailAndPassword,
  signInWithPopup,
  signInWithRedirect,
  signOut,
  type User,
} from "firebase/auth";
import {type FormEvent, useEffect, useState} from "react";

import {clientAuth} from "@/lib/firebase-client";

/** `agent-graph watch-remote`, waiting on this computer for the login. */
export type CliLogin = {port: number; state: string; challenge: string};

type Mode = "log-in" | "sign-up" | "reset";

/**
 * Logging in, with Google or an email and password (Firebase
 * Authentication, in the browser), then trading the sign-in for the site's
 * session cookie (/api/session). For the CLI, once logged in, it asks to
 * connect it (/api/cli/code), then sends the browser to it.
 */
export function LoginForm({
  next,
  cli,
  badCli,
  email: loggedInAs,
}: {
  next: string;
  cli: CliLogin | null;
  badCli: boolean;
  /** Who's logged in already, if anyone. */
  email: string | null;
}) {
  const [mode, setMode] = useState<Mode>("log-in");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [account, setAccount] = useState<string | null>(loggedInAs);

  // Back from a Google sign-in that went by redirect (a popup was blocked).
  useEffect(() => {
    getRedirectResult(clientAuth())
      .then(result => result && finish(result.user))
      .catch(err => setError(message(err)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  /** The sign-in, as the site's session; then on, to the CLI or `next`. */
  async function finish(user: User) {
    setBusy(true);
    setError(null);
    try {
      const idToken = await user.getIdToken();
      const res = await fetch("/api/session", {
        method: "POST",
        headers: {"Content-Type": "application/json"},
        body: JSON.stringify({idToken}),
      });
      // The session cookie is the record of it from here on.
      await signOut(clientAuth()).catch(() => {});
      if (!res.ok) {
        throw new Error(await res.text());
      }
      if (cli) {
        setAccount(user.email ?? "");
        await connect();
      } else {
        location.assign(next);
      }
    } catch (err) {
      setError(message(err));
      setBusy(false);
    }
  }

  /** Hands the login to the CLI waiting on this computer. */
  async function connect() {
    setBusy(true);
    setError(null);
    try {
      const res = await fetch("/api/cli/code", {
        method: "POST",
        headers: {"Content-Type": "application/json"},
        body: JSON.stringify(cli),
      });
      if (res.status === 401) {
        setAccount(null);
        throw new Error("Log in first.");
      }
      if (!res.ok) {
        throw new Error(await res.text());
      }
      const {redirect} = (await res.json()) as {redirect: string};
      location.assign(redirect);
    } catch (err) {
      setError(message(err));
      setBusy(false);
    }
  }

  async function google() {
    setBusy(true);
    setError(null);
    const provider = new GoogleAuthProvider();
    try {
      const result = await signInWithPopup(clientAuth(), provider);
      await finish(result.user);
    } catch (err) {
      const code = (err as {code?: string}).code;
      if (code === "auth/popup-blocked") {
        await signInWithRedirect(clientAuth(), provider);
        return;
      }
      if (
        code !== "auth/popup-closed-by-user" &&
        code !== "auth/cancelled-popup-request"
      ) {
        setError(message(err));
      }
      setBusy(false);
    }
  }

  async function submit(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      if (mode === "reset") {
        await sendPasswordResetEmail(clientAuth(), email);
        setNotice(
          `If there's an account for ${email}, an email is on its way with a link to choose a new password.`,
        );
        setMode("log-in");
        setBusy(false);
        return;
      }
      const result =
        mode === "sign-up"
          ? await createUserWithEmailAndPassword(clientAuth(), email, password)
          : await signInWithEmailAndPassword(clientAuth(), email, password);
      await finish(result.user);
    } catch (err) {
      setError(message(err));
      setBusy(false);
    }
  }

  async function useAnother() {
    await fetch("/api/session", {method: "DELETE"}).catch(() => {});
    setAccount(null);
  }

  const heading = cli
    ? "Connect agent-graph"
    : mode === "sign-up"
      ? "Create an account"
      : "Log in";

  return (
    <div className="card narrow">
      <div className="card-body">
        <h1>{heading}</h1>
        {badCli ? (
          <p className="error">
            This link from agent-graph isn&rsquo;t complete. Run{" "}
            <code>agent-graph watch-remote</code> again.
          </p>
        ) : null}
        {cli ? (
          <p>
            <code>agent-graph watch-remote</code> is asking to share your
            agents&rsquo; activity to your account. Only you will be able to see
            it, at <a href="/watch">/watch</a>.
          </p>
        ) : null}

        {cli && account !== null ? (
          <div className="auth-choice">
            <p>
              You&rsquo;re logged in as{" "}
              <strong>{account || "your account"}</strong>.
            </p>
            <button
              className="button"
              type="button"
              onClick={connect}
              disabled={busy}>
              {busy ? "Connecting…" : "Connect agent-graph"}
            </button>
            <button
              className="link-button"
              type="button"
              onClick={useAnother}
              disabled={busy}>
              Use another account
            </button>
          </div>
        ) : (
          <div className="auth-choice">
            {mode !== "reset" ? (
              <>
                <button
                  className="button secondary"
                  type="button"
                  onClick={google}
                  disabled={busy}>
                  Continue with Google
                </button>
                <div className="or">or</div>
              </>
            ) : null}
            <form className="auth-choice" onSubmit={submit}>
              <label className="field">
                Email
                <input
                  type="email"
                  autoComplete="email"
                  required
                  value={email}
                  onChange={e => setEmail(e.target.value)}
                />
              </label>
              {mode !== "reset" ? (
                <label className="field">
                  Password
                  <input
                    type="password"
                    autoComplete={
                      mode === "sign-up" ? "new-password" : "current-password"
                    }
                    required
                    minLength={mode === "sign-up" ? 6 : undefined}
                    value={password}
                    onChange={e => setPassword(e.target.value)}
                  />
                </label>
              ) : null}
              <button className="button" type="submit" disabled={busy}>
                {mode === "reset"
                  ? "Send a reset link"
                  : mode === "sign-up"
                    ? "Create account"
                    : "Log in"}
              </button>
            </form>
            <div className="auth-switch">
              {mode === "log-in" ? (
                <>
                  <button
                    className="link-button"
                    type="button"
                    onClick={() => setMode("sign-up")}>
                    Create an account
                  </button>
                  <button
                    className="link-button"
                    type="button"
                    onClick={() => setMode("reset")}>
                    Forgot your password?
                  </button>
                </>
              ) : (
                <button
                  className="link-button"
                  type="button"
                  onClick={() => setMode("log-in")}>
                  {mode === "sign-up"
                    ? "I have an account: log in"
                    : "Back to logging in"}
                </button>
              )}
            </div>
          </div>
        )}
        {notice ? <p className="notice">{notice}</p> : null}
        {error ? <p className="error">{error}</p> : null}
      </div>
    </div>
  );
}

/** What went wrong, for people: Firebase's errors are codes. */
function message(err: unknown): string {
  const code = (err as {code?: string}).code ?? "";
  switch (code) {
    case "auth/invalid-credential":
    case "auth/wrong-password":
    case "auth/user-not-found":
      return "That email and password don't match an account.";
    case "auth/email-already-in-use":
      return "There's already an account with that email. Log in instead.";
    case "auth/weak-password":
      return "Choose a password of at least 6 characters.";
    case "auth/invalid-email":
      return "That doesn't look like an email address.";
    case "auth/too-many-requests":
      return "Too many attempts. Wait a few minutes, and try again.";
    case "auth/network-request-failed":
      return "Couldn't reach the sign-in service. Check your connection, and try again.";
    case "auth/account-exists-with-different-credential":
      return "That email already has an account, with another way of logging in. Use that.";
    case "auth/popup-blocked":
      return "Your browser blocked the sign-in window.";
    default:
      return err instanceof Error && err.message
        ? err.message
        : "That didn't work. Try again.";
  }
}
