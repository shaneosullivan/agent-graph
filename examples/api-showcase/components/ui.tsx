"use client";

import {
  STATES,
  STALE_COLOR,
  BLOCKED_COLOR,
  nodeName,
  nodeLine,
  ago,
} from "@/lib/format";
import type {AgentNode, NodeState} from "@/lib/types";
import {AgentGraphError} from "@/lib/client";

/** A view's head: what it shows, why that's worth having, and the calls it makes. */
export function Explainer({
  kicker,
  title,
  lede,
  what,
  why,
  calls,
}: {
  kicker: string;
  title: string;
  lede: React.ReactNode;
  what: React.ReactNode;
  why: React.ReactNode;
  calls: Array<string>;
}) {
  return (
    <header className="explainer">
      <div className="kicker">{kicker}</div>
      <h1>{title}</h1>
      <p className="lede">{lede}</p>
      <div className="box">
        <h3>What you&rsquo;re looking at</h3>
        {what}
      </div>
      <div className="box">
        <h3>Why it matters</h3>
        {why}
      </div>
      <div className="calls-used">
        <span>API calls:</span>
        {calls.map(c => (
          <code key={c}>{c}</code>
        ))}
      </div>
    </header>
  );
}

export function StateBadge({state}: {state: NodeState}) {
  const s = STATES[state];
  return (
    <span
      className="badge"
      style={{"--c": s.color} as React.CSSProperties}
      title={s.hint}>
      <span className="dot" />
      {s.label}
    </span>
  );
}

export function Flag({kind}: {kind: "stale" | "blocked"}) {
  const c = kind === "stale" ? STALE_COLOR : BLOCKED_COLOR;
  return (
    <span className="badge" style={{"--c": c} as React.CSSProperties}>
      {kind === "stale" ? "⚠ silent too long" : "⧗ waiting on others"}
    </span>
  );
}

export function Loading({what = "Asking the API"}: {what?: string}) {
  return (
    <div className="loading">
      <span className="spinner" /> {what}…
    </div>
  );
}

export function ErrorBox({error}: {error: Error}) {
  const e = error instanceof AgentGraphError ? error : null;
  return (
    <div className="error-box">
      <b>{e ? `${e.status} · ${e.code}` : "Something went wrong"}</b>
      <div style={{marginTop: 4}}>{error.message}</div>
      {e?.code === "api_key_missing" || e?.code === "api_key_invalid" ? (
        <div style={{marginTop: 8}} className="muted">
          Check <code>AGENT_GRAPH_KEY</code> in this app&rsquo;s{" "}
          <code>.env</code>, then restart it.
        </div>
      ) : null}
    </div>
  );
}

export function Tile({
  label,
  value,
  note,
  color,
}: {
  label: string;
  value: React.ReactNode;
  note?: React.ReactNode;
  color?: string;
}) {
  return (
    <div
      className="tile"
      style={{"--glow": color ?? "var(--accent)"} as React.CSSProperties}>
      <div className="label">{label}</div>
      <div className="value" style={color ? {color} : undefined}>
        {value}
      </div>
      {note ? <div className="note">{note}</div> : null}
    </div>
  );
}

/** A node as a row: its state, name, what it's doing, and how long since it was heard from. */
export function NodeRow({
  node,
  onClick,
  right,
}: {
  node: AgentNode;
  onClick?: () => void;
  right?: React.ReactNode;
}) {
  return (
    <div className="list-item" onClick={onClick}>
      <span
        style={{
          width: 10,
          height: 10,
          marginTop: 7,
          borderRadius: "50%",
          background: STATES[node.state].color,
          boxShadow: `0 0 10px ${STATES[node.state].color}`,
          flex: "none",
        }}
      />
      <div style={{minWidth: 0, flex: 1}}>
        <div className="row" style={{gap: 8}}>
          <span className="title">{nodeName(node)}</span>
          <span className="faint" style={{fontSize: 12}}>
            {node.kind} · {node.provider} · {ago(node.last_event)}
          </span>
        </div>
        {nodeLine(node) ? <div className="line">{nodeLine(node)}</div> : null}
      </div>
      {right}
    </div>
  );
}

export function Legend({
  states = true,
  flags = false,
}: {
  states?: boolean;
  flags?: boolean;
}) {
  return (
    <div className="legend">
      {states
        ? Object.entries(STATES).map(([k, s]) => (
            <span key={k}>
              <i style={{background: s.color}} /> {s.label}
            </span>
          ))
        : null}
      {flags ? (
        <>
          <span>
            <i
              style={{
                background: "transparent",
                border: `2px dashed ${STALE_COLOR}`,
                borderRadius: "50%",
              }}
            />{" "}
            silent too long (stale)
          </span>
          <span>
            <i style={{background: BLOCKED_COLOR, borderRadius: "50%"}} />{" "}
            waiting on another node
          </span>
        </>
      ) : null}
    </div>
  );
}
