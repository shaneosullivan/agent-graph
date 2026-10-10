"use client";

import {useApi, useLoad} from "@/lib/client";
import {now, ago, clock, duration, nodeName, ranFor, STATES} from "@/lib/format";
import type {AgentNode} from "@/lib/types";

import {ErrorBox, Flag, Loading, StateBadge} from "./ui";

const EXPAND = [
  "parent",
  "tasks",
  "spawns",
  "waits",
  "messages",
  "blocked.on",
  "children",
];

/**
 * One node, in full: retrieved with everything around it expanded in the
 * same call (its parent, tasks, spawns, waits, messages, what it's waiting
 * on, its children). Each id links to that node.
 */
export function NodeDrawer({
  graph,
  nodeId,
  asOf,
  onSelect,
  onClose,
}: {
  graph: string;
  nodeId: string;
  asOf?: string | null;
  onSelect: (id: string) => void;
  onClose?: () => void;
}) {
  const {get} = useApi();
  const {
    data: n,
    error,
    loading,
  } = useLoad(
    async () =>
      (
        await get<AgentNode>(`/graphs/${graph}/nodes/${nodeId}`, {
          expand: EXPAND,
          ...(asOf ? {as_of: asOf} : {}),
        })
      ).data!,
    [graph, nodeId, asOf],
  );

  return (
    <aside className="panel drawer">
      <div className="row">
        <span
          className="faint"
          style={{
            fontSize: 12,
            letterSpacing: "0.08em",
            textTransform: "uppercase",
          }}>
          Node
        </span>
        <div className="spacer" />
        {onClose ? (
          <button
            className="btn"
            style={{padding: "2px 10px"}}
            onClick={onClose}
            aria-label="Close">
            ✕
          </button>
        ) : null}
      </div>
      {loading && !n ? (
        <Loading what="Retrieving the node, with what's around it" />
      ) : null}
      {error ? <ErrorBox error={error} /> : null}
      {n ? (
        <>
          <h2
            style={{
              fontSize: 20,
              margin: "6px 0 8px",
              letterSpacing: "-0.01em",
            }}>
            {nodeName(n)}
          </h2>
          <div className="row" style={{gap: 6}}>
            <StateBadge state={n.state} />
            {n.stale ? <Flag kind="stale" /> : null}
            {n.blocked ? <Flag kind="blocked" /> : null}
            <span className="chip" style={{cursor: "default"}}>
              {n.kind} · {n.provider}
              {n.agent_type ? ` · ${n.agent_type}` : ""}
            </span>
          </div>
          {n.attention && n.state === "input_required" ? (
            <div className="callout warn" style={{marginTop: 12}}>
              <span>✋</span>
              <span>
                <b>Asking:</b> {n.attention}
              </span>
            </div>
          ) : null}
          {n.headline || n.purpose ? (
            <p style={{margin: "12px 0 0", color: "#c8cde0"}}>
              {n.headline ?? n.purpose}
            </p>
          ) : null}
          {n.purpose && n.headline ? (
            <p className="muted" style={{margin: "6px 0 0", fontSize: 13}}>
              Started to: {n.purpose}
            </p>
          ) : null}

          <h3>Timing</h3>
          <dl className="kv">
            <dt>Started</dt>
            <dd>{clock(n.created)}</dd>
            <dt>{n.ended ? "Ended" : "Last heard"}</dt>
            <dd>
              {clock(n.ended ?? n.last_event)}{" "}
              <span className="faint">({ago(n.ended ?? n.last_event)})</span>
            </dd>
            <dt>Ran for</dt>
            <dd>{duration(ranFor(n))}</dd>
            <dt>Depth</dt>
            <dd>
              {n.depth} · {n.child_count}{" "}
              {n.child_count === 1 ? "child" : "children"} ·{" "}
              {n.descendant_count} below
            </dd>
          </dl>

          {n.blocked ? (
            <>
              <h3>Waiting on</h3>
              <p className="muted" style={{fontSize: 13, margin: "0 0 6px"}}>
                {n.blocked.node_count} unfinished node
                {n.blocked.node_count === 1 ? "" : "s"} and{" "}
                {n.blocked.open_tasks} open task
                {n.blocked.open_tasks === 1 ? "" : "s"} stand between it and
                carrying on
                {n.blocked.cycle ? (
                  <b style={{color: "var(--bad)"}}>
                    {" "}
                    — and the waits form a loop: nothing in it can finish.
                  </b>
                ) : (
                  "."
                )}
              </p>
              {(n.blocked.on ?? []).map(o => (
                <LinkRow key={o.id} node={o} onSelect={onSelect} />
              ))}
            </>
          ) : null}

          {n.tasks?.length ? (
            <>
              <h3>
                Tasks · {n.task_counts.completed}/{n.task_counts.total} done
              </h3>
              <div className="progress" style={{marginBottom: 8}}>
                <div
                  style={{
                    width: `${(100 * n.task_counts.completed) / n.task_counts.total}%`,
                  }}
                />
              </div>
              {n.tasks.map(t => (
                <div key={t.id} className="task">
                  <span
                    style={{
                      color:
                        t.status === "completed"
                          ? "var(--good)"
                          : t.status === "in_progress"
                            ? "var(--accent)"
                            : "var(--faint)",
                    }}>
                    {t.status === "completed"
                      ? "✓"
                      : t.status === "in_progress"
                        ? "▸"
                        : "○"}
                  </span>
                  <span
                    style={{
                      color:
                        t.status === "completed" ? "var(--muted)" : undefined,
                    }}>
                    {t.status === "in_progress"
                      ? (t.active_text ?? t.text)
                      : t.text}
                  </span>
                </div>
              ))}
            </>
          ) : null}

          {n.parent ? (
            <>
              <h3>Started by</h3>
              <LinkRow node={n.parent} onSelect={onSelect} />
            </>
          ) : null}

          {n.children?.data.length ? (
            <>
              <h3>
                Started {n.child_count} {n.child_count === 1 ? "node" : "nodes"}
              </h3>
              {n.children.data.map(c => (
                <LinkRow key={c.id} node={c} onSelect={onSelect} />
              ))}
            </>
          ) : null}

          {n.waits?.length ? (
            <>
              <h3>Waits</h3>
              {n.waits.map(w => (
                <div key={w.id} className="task">
                  <span style={{color: w.open ? BLOCKED : "var(--faint)"}}>
                    {w.open ? "⧗" : "✓"}
                  </span>
                  <span className={w.open ? undefined : "muted"}>
                    {w.kind === "spawn"
                      ? "for a child it asked for"
                      : "on another node"}
                    {w.reason ? `: ${w.reason}` : ""}{" "}
                    <span className="faint">
                      (
                      {w.ended
                        ? `waited ${duration(w.ended - w.started)}`
                        : `${duration(now() - w.started)} so far`}
                      )
                    </span>
                  </span>
                </div>
              ))}
            </>
          ) : null}

          {n.messages?.length ? (
            <>
              <h3>Messages</h3>
              {n.messages.map(m => (
                <div key={m.id} className="task">
                  <span className="faint">
                    {m.direction === "sent" ? "→" : "←"}
                  </span>
                  <span>
                    {m.summary ?? "(no summary)"}{" "}
                    {m.peer_id ? (
                      <a
                        href="#"
                        onClick={e => (
                          e.preventDefault(),
                          onSelect(m.peer_id!)
                        )}>
                        open the other side
                      </a>
                    ) : (
                      <span className="faint">{m.peer_name}</span>
                    )}
                  </span>
                </div>
              ))}
            </>
          ) : null}

          <h3>Ids, never objects</h3>
          <p className="muted" style={{fontSize: 12.5, margin: "0 0 6px"}}>
            Every field ending <code>_id</code> is always there, as an id. The
            object itself (<code>parent</code>, <code>blocked.on</code>…)
            appears beside it only because this call asked for it with{" "}
            <code>expand[]</code>.
          </p>
          <dl className="kv mono" style={{fontSize: 11.5}}>
            <dt>id</dt>
            <dd>{n.id}</dd>
            <dt>parent_id</dt>
            <dd>{n.parent_id ?? "null"}</dd>
            <dt>root_id</dt>
            <dd>{n.root_id}</dd>
            <dt>session_id</dt>
            <dd>{n.session_id}</dd>
            <dt>provider_ref</dt>
            <dd>{n.provider_ref}</dd>
          </dl>
        </>
      ) : null}
    </aside>
  );
}

const BLOCKED = "var(--blocked)";

function LinkRow({
  node,
  onSelect,
}: {
  node: AgentNode;
  onSelect: (id: string) => void;
}) {
  return (
    <div
      className="task"
      style={{cursor: "pointer", alignItems: "center"}}
      onClick={() => onSelect(node.id)}>
      <span
        style={{
          width: 8,
          height: 8,
          borderRadius: "50%",
          background: STATES[node.state].color,
          flex: "none",
        }}
      />
      <span style={{color: "var(--accent)"}}>{nodeName(node)}</span>
      <span className="faint" style={{fontSize: 12}}>
        {STATES[node.state].label}
      </span>
    </div>
  );
}
