"use client";

import {STATES, nodeName} from "@/lib/format";
import type {AgentNode} from "@/lib/types";
import {useWidth} from "@/lib/useWidth";

import {useTooltip} from "./tooltip";

const BOX_W = 186;
const BOX_H = 38;
const GAP_X = 18;
const GAP_Y = 64;

/**
 * Who's waiting on whom, top to bottom: waiting nodes above, what they wait
 * on below them, and what everything's waiting for at the bottom. A loop of
 * waits, which can never finish, is drawn in red.
 */
export function WaitGraph({
  waiting,
  onPick,
}: {
  /** Nodes that are blocked, each with `blocked.on` expanded. */
  waiting: Array<AgentNode>;
  onPick?: (id: string) => void;
}) {
  const {tip, show, hide} = useTooltip();
  const {ref, width} = useWidth();
  const nodes = new Map<string, AgentNode>();
  const edges: Array<{from: string; to: string}> = [];
  for (const w of waiting) {
    nodes.set(w.id, w);
    for (const o of w.blocked?.on ?? []) {
      if (!nodes.has(o.id)) nodes.set(o.id, o);
      edges.push({from: w.id, to: o.id});
    }
  }
  if (!edges.length) {
    return (
      <div ref={ref} className="empty">
        Nothing is waiting on anything right now. 🎉
      </div>
    );
  }

  // A node's level: how far it is from what everything's waiting for (the
  // longest chain of waits below it; a loop counts once).
  const out = new Map<string, Array<string>>();
  for (const e of edges) out.set(e.from, [...(out.get(e.from) ?? []), e.to]);
  const memo = new Map<string, number>();
  const level = (id: string, seen: Set<string>): number => {
    if (memo.has(id)) return memo.get(id)!;
    if (seen.has(id)) return 0;
    seen.add(id);
    const l = Math.max(-1, ...(out.get(id) ?? []).map(t => level(t, seen))) + 1;
    seen.delete(id);
    memo.set(id, l);
    return l;
  };
  const ids = [...nodes.keys()];
  const top = Math.max(...ids.map(id => level(id, new Set())));
  const rows: Array<Array<string>> = Array.from({length: top + 1}, () => []);
  for (const id of ids) rows[top - level(id, new Set())].push(id);

  // Each row of nodes wraps to the width there is.
  const perLine = Math.max(1, Math.floor((width + GAP_X) / (BOX_W + GAP_X)));
  const pos = new Map<string, {x: number; y: number}>();
  let y = 8;
  for (const row of rows) {
    const lines = Math.ceil(row.length / perLine);
    row.forEach((id, k) => {
      const line = Math.floor(k / perLine);
      const inLine = Math.min(perLine, row.length - line * perLine);
      const span = inLine * BOX_W + (inLine - 1) * GAP_X;
      const x0 = (width - span) / 2;
      pos.set(id, {
        x: x0 + (k % perLine) * (BOX_W + GAP_X),
        y: y + line * (BOX_H + 18),
      });
    });
    y += lines * (BOX_H + 18) - 18 + GAP_Y;
  }
  const height = y - GAP_Y + 16;

  const cyclic = new Set(waiting.filter(w => w.blocked?.cycle).map(w => w.id));
  const waitedOn = new Map<string, number>();
  for (const e of edges) waitedOn.set(e.to, (waitedOn.get(e.to) ?? 0) + 1);

  return (
    <div ref={ref}>
      <svg width={width} height={height} className="chart">
        <defs>
          <marker
            id="arrow"
            viewBox="0 0 10 10"
            refX="9"
            refY="5"
            markerWidth="7"
            markerHeight="7"
            orient="auto">
            <path d="M0,0 L10,5 L0,10 z" fill="var(--blocked)" />
          </marker>
          <marker
            id="arrow-red"
            viewBox="0 0 10 10"
            refX="9"
            refY="5"
            markerWidth="7"
            markerHeight="7"
            orient="auto">
            <path d="M0,0 L10,5 L0,10 z" fill="var(--bad)" />
          </marker>
        </defs>
        {edges.map((e, i) => {
          const a = pos.get(e.from)!;
          const b = pos.get(e.to)!;
          const loop = cyclic.has(e.from) && cyclic.has(e.to);
          let d: string;
          if (b.y > a.y) {
            // Down to what it waits on.
            const x1 = a.x + BOX_W / 2;
            const y1 = a.y + BOX_H;
            const x2 = b.x + BOX_W / 2;
            const y2 = b.y - 2;
            d = `M${x1},${y1} C${x1},${(y1 + y2) / 2} ${x2},${(y1 + y2) / 2} ${x2},${y2}`;
          } else {
            // Sideways or back up (a loop): out of one side, round, into the other.
            const x1 = a.x + BOX_W;
            const y1 = a.y + BOX_H / 2;
            const x2 = b.x + BOX_W + 2;
            const y2 = b.y + BOX_H / 2;
            const bulge = Math.max(x1, x2) + 46;
            d = `M${x1},${y1} C${bulge},${y1} ${bulge},${y2} ${x2},${y2}`;
          }
          return (
            <path
              key={i}
              d={d}
              fill="none"
              stroke={loop ? "var(--bad)" : "var(--blocked)"}
              strokeOpacity={0.75}
              strokeWidth={1.6}
              strokeDasharray={loop ? "5 4" : undefined}
              markerEnd={`url(#${loop ? "arrow-red" : "arrow"})`}
            />
          );
        })}
        {ids.map(id => {
          const n = nodes.get(id)!;
          const p = pos.get(id)!;
          const color = STATES[n.state].color;
          const behind = waitedOn.get(id) ?? 0;
          return (
            <g
              key={id}
              transform={`translate(${p.x},${p.y})`}
              style={{cursor: "pointer"}}
              onClick={() => onPick?.(id)}
              onMouseMove={e =>
                show(
                  e,
                  <>
                    <b>{nodeName(n)}</b>
                    <div className="muted">{STATES[n.state].label}</div>
                    {n.blocked ? (
                      <div>
                        waiting on {n.blocked.node_count} node
                        {n.blocked.node_count === 1 ? "" : "s"},{" "}
                        {n.blocked.open_tasks} open task
                        {n.blocked.open_tasks === 1 ? "" : "s"}
                      </div>
                    ) : null}
                    {behind ? <div>{behind} waiting on it directly</div> : null}
                  </>,
                )
              }
              onMouseLeave={hide}>
              <rect
                width={BOX_W}
                height={BOX_H}
                rx={10}
                fill="var(--panel-2)"
                stroke={cyclic.has(id) ? "var(--bad)" : color}
                strokeOpacity={0.85}
              />
              <circle cx={14} cy={BOX_H / 2} r={5} fill={color} />
              <text x={26} y={16} style={{fill: "var(--text)", fontSize: 12}}>
                {nodeName(n).slice(0, 24)}
              </text>
              <text x={26} y={30} style={{fontSize: 10.5}}>
                {[
                  n.blocked ? `waits on ${n.blocked.on_ids.length}` : "",
                  behind ? `${behind} waiting on it` : "",
                ]
                  .filter(Boolean)
                  .join(" · ")}
              </text>
            </g>
          );
        })}
      </svg>
      {tip}
    </div>
  );
}
