"use client";

import {useRouter} from "next/navigation";
import {useState} from "react";

import {CopyCommand} from "../copy-command";
import {KeyIcon} from "./icons";

/** A key, as the account page lists it (app/api/account/api-keys). */
export type ShownKey = {
  id: string;
  name: string;
  kind: "secret" | "restricted";
  /** For a restricted key, the graphs it reads (`gph_…`). */
  graphs: Array<string> | null;
  shown: string;
  createdAt: number;
  usedAt: number | null;
};

/** A live share a restricted key can be made for. */
export type KeyableShare = {id: string; host: string};

const date = (ms: number) =>
  new Date(ms).toLocaleDateString("en-GB", {
    day: "numeric",
    month: "short",
    year: "numeric",
    timeZone: "UTC",
  });

/**
 * Keys to the graph API (lib/api/keys.ts): read-only, for the account's
 * own servers and the AI systems they run. A secret key reads every graph;
 * a restricted one, only the live shares chosen for it. Each is shown
 * once, when it's made, and can be revoked.
 */
export function ApiKeys({
  keys,
  shares,
}: {
  keys: Array<ShownKey>;
  shares: Array<KeyableShare>;
}) {
  const router = useRouter();
  const [name, setName] = useState("");
  const [kind, setKind] = useState<"secret" | "restricted">("secret");
  const [chosen, setChosen] = useState<Array<string>>([]);
  const [made, setMade] = useState<{name: string; key: string} | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const graphOf = (share: string) => `gph_${share}`;
  const toggle = (graph: string) =>
    setChosen(c =>
      c.includes(graph) ? c.filter(g => g !== graph) : [...c, graph],
    );

  async function create(event: React.FormEvent) {
    event.preventDefault();
    setBusy("new");
    setError(null);
    try {
      const res = await fetch("/api/account/api-keys", {
        method: "POST",
        headers: {"Content-Type": "application/json"},
        body: JSON.stringify(
          kind === "restricted" ? {name, kind, graphs: chosen} : {name, kind},
        ),
      });
      if (!res.ok) {
        throw new Error(await res.text());
      }
      const {key} = (await res.json()) as {key: string};
      setMade({name: name.trim(), key});
      setName("");
      setChosen([]);
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
    const res = await fetch(`/api/account/api-keys/${id}`, {method: "DELETE"});
    if (!res.ok) {
      setError(await res.text());
    }
    setBusy(null);
    router.refresh();
  }

  const ready =
    name.trim() && (kind === "secret" || chosen.length > 0) && busy !== "new";

  return (
    <>
      {made ? (
        <div className="token-made" role="status">
          <p>
            <strong>Your key for &ldquo;{made.name}&rdquo;.</strong> Copy it
            now: it won&rsquo;t be shown again.
          </p>
          <CopyCommand command={`export AGENT_GRAPH_KEY=${made.key}`} />
          <p>
            Then read your graphs with it: see the{" "}
            <a href="/docs/reference">API reference</a>.
          </p>
          <button
            className="link-button"
            type="button"
            onClick={() => setMade(null)}>
            Done
          </button>
        </div>
      ) : null}
      {keys.length ? (
        <ul className="acct-rows">
          {keys.map(k => (
            <li key={k.id}>
              <span className="acct-row-icon">
                <KeyIcon />
              </span>
              <span className="acct-row-main">
                <strong>{k.name || "An API key"}</strong>
                <span className="acct-row-sub">
                  <code>{k.shown}</code> ·{" "}
                  {k.kind === "secret"
                    ? "reads every graph"
                    : `reads ${k.graphs?.length ?? 0} graph${k.graphs?.length === 1 ? "" : "s"}`}{" "}
                  · made {date(k.createdAt)} ·{" "}
                  {k.usedAt ? `last used ${date(k.usedAt)}` : "not used yet"}
                </span>
              </span>
              <button
                className="acct-row-action"
                type="button"
                onClick={() => revoke(k.id)}
                disabled={busy === k.id}>
                {busy === k.id ? "Revoking…" : "Revoke"}
              </button>
            </li>
          ))}
        </ul>
      ) : null}
      <form className="token-form key-form" onSubmit={create}>
        <label className="field">
          <span>Name it for what will use it</span>
          <input
            type="text"
            placeholder="e.g. optimizer"
            maxLength={100}
            value={name}
            onChange={e => setName(e.target.value)}
            disabled={busy === "new"}
          />
        </label>
        <fieldset className="key-kind" disabled={busy === "new"}>
          <legend>It can read</legend>
          <label>
            <input
              type="radio"
              name="kind"
              checked={kind === "secret"}
              onChange={() => setKind("secret")}
            />{" "}
            Every graph
          </label>
          <label>
            <input
              type="radio"
              name="kind"
              checked={kind === "restricted"}
              onChange={() => setKind("restricted")}
              disabled={!shares.length}
            />{" "}
            Only the live shares I choose
          </label>
          {kind === "restricted"
            ? shares.map(s => (
                <label key={s.id} className="key-graph">
                  <input
                    type="checkbox"
                    checked={chosen.includes(graphOf(s.id))}
                    onChange={() => toggle(graphOf(s.id))}
                  />{" "}
                  {s.host || "A live share"} <code>{graphOf(s.id)}</code>
                </label>
              ))
            : null}
        </fieldset>
        <button className="button secondary" type="submit" disabled={!ready}>
          {busy === "new" ? "Making…" : "Make a key"}
        </button>
      </form>
      {error ? <p className="error">{error}</p> : null}
    </>
  );
}
