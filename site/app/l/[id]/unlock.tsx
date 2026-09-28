"use client";

import "../../site.css";

import { useRouter } from "next/navigation";
import { useState } from "react";

import { SiteHeader } from "../../site-header";

export function Unlock({ id }: { id: string }) {
  const router = useRouter();
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    const res = await fetch(`/api/logs/${id}/unlock`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ password }),
    });
    if (res.ok) {
      router.refresh();
      return;
    }
    setError(res.status === 401 ? "That's not the password." : await res.text());
    setBusy(false);
  }

  return (
    <div className="site">
      <div className="page">
        <SiteHeader />
        <form className="card narrow" onSubmit={submit}>
          <div className="card-body">
            <h1>This log is password protected</h1>
            <p>Enter the password you were given to view it.</p>
            <label className="field">
              Password
              <input
                type="password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                autoFocus
                autoComplete="current-password"
                required
              />
            </label>
            <button className="button" type="submit" disabled={busy || !password}>
              {busy ? "Checking…" : "View log"}
            </button>
            {error && <p className="error">{error}</p>}
          </div>
        </form>
      </div>
    </div>
  );
}
