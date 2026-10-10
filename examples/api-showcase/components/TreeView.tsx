"use client";

import * as d3 from "d3";
import {useEffect, useMemo, useState} from "react";

import {
  BLOCKED_COLOR,
  STALE_COLOR,
  STATES,
  nodeLine,
  nodeName,
} from "@/lib/format";
import type {AgentNode} from "@/lib/types";

type Item = {node: AgentNode | null; id: string; kids: Array<Item>};

const ROW = 34;
const COL = 250;
/** Room above the first node, for a label drawn above it. */
const TOP = 16;
/** How much of a name, and of what it's doing, a label shows. */
const NAME_CHARS = 28;
const SUB_CHARS = 32;

/** Where a node's label ends, roughly (so lines leave from there, not through it). */
function labelEnd(n: AgentNode): number {
  const chars = Math.max(
    Math.min(nodeName(n).length, NAME_CHARS) * 6.9,
    Math.min(nodeLine(n).length, SUB_CHARS) * 6.1,
  );
  return Math.min(COL - 30, radius(n) + 14 + chars);
}

/** Nodes as a forest: each under its parent (by `parent_id`), oldest first, as the API lists them in tree order. */
export function forestOf(nodes: Array<AgentNode>): Item {
  const byId = new Map(
    nodes.map(n => [n.id, {node: n, id: n.id, kids: [] as Array<Item>}]),
  );
  const top: Array<Item> = [];
  for (const n of nodes) {
    const item = byId.get(n.id)!;
    const parent = n.parent_id ? byId.get(n.parent_id) : undefined;
    (parent ? parent.kids : top).push(item);
  }
  return {node: null, id: "__forest", kids: top};
}

const radius = (n: AgentNode) =>
  Math.min(16, 5 + Math.sqrt(n.descendant_count) * 1.8);

/**
 * A tree of nodes, left to right. Click a node's dot to fold or unfold what's
 * under it; click its name to pick it. `flash` marks nodes that just changed.
 */
export function TreeView({
  nodes,
  selected,
  onSelect,
  flash,
  query = "",
  foldBelow,
  height = 620,
  pending,
}: {
  nodes: Array<AgentNode>;
  selected?: string | null;
  onSelect?: (n: AgentNode) => void;
  flash?: Set<string>;
  query?: string;
  /** Fold everything deeper than this, to start with. */
  foldBelow?: number;
  height?: number;
  /** Nodes about to change (dimmed while a new moment loads). */
  pending?: boolean;
}) {
  const forest = useMemo(() => forestOf(nodes), [nodes]);
  const [folded, setFolded] = useState<Set<string>>(new Set());
  const [zoom, setZoom] = useState(1);

  // Start folded below a depth, each time a different tree arrives.
  const rootKey = forest.kids.map(k => k.id).join(",");
  useEffect(() => {
    if (foldBelow === undefined) return;
    const f = new Set<string>();
    for (const n of nodes) {
      if (n.depth >= foldBelow && n.child_count > 0) f.add(n.id);
    }
    setFolded(f);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [rootKey, foldBelow]);

  const layout = useMemo(() => {
    const root = d3.hierarchy<Item>(forest, d =>
      d.id !== "__forest" && folded.has(d.id) ? undefined : d.kids,
    );
    const tree = d3
      .tree<Item>()
      .nodeSize([ROW, COL])
      .separation((a, b) => (a.parent === b.parent ? 1 : 1.25));
    const laid = tree(root);
    let minX = Infinity;
    let maxX = -Infinity;
    let maxY = 0;
    laid.each(d => {
      minX = Math.min(minX, d.x);
      maxX = Math.max(maxX, d.x);
      maxY = Math.max(maxY, d.y);
    });
    return {laid, minX, maxX, maxY};
  }, [forest, folded]);

  const q = query.trim().toLowerCase();
  const matches = (n: AgentNode) =>
    !q ||
    `${nodeName(n)} ${nodeLine(n)} ${n.agent_type ?? ""} ${n.state}`
      .toLowerCase()
      .includes(q);

  const width = layout.maxY + COL + 40;
  const svgHeight = layout.maxX - layout.minX + ROW * 2 + TOP;
  const at = (d: d3.HierarchyPointNode<Item>): [number, number] => [
    d.y - COL + 30,
    d.x - layout.minX + ROW + TOP,
  ];
  const link = (l: d3.HierarchyPointLink<Item>) => {
    const [x0, y0] = at(l.source);
    const [x1, y1] = at(l.target);
    const from = x0 + labelEnd(l.source.data.node!);
    const to = x1 - radius(l.target.data.node!) - 2;
    const mid = (from + to) / 2;
    return `M${from},${y0} C${mid},${y0} ${mid},${y1} ${to},${y1}`;
  };

  const toggle = (id: string) =>
    setFolded(f => {
      const next = new Set(f);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  const all = layout.laid.descendants().filter(d => d.data.node);

  return (
    <div>
      <div className="row" style={{marginBottom: 10}}>
        <button className="btn" onClick={() => setFolded(new Set())}>
          Unfold all
        </button>
        <button
          className="btn"
          onClick={() =>
            setFolded(
              new Set(
                nodes
                  .filter(n => n.depth >= 1 && n.child_count > 0)
                  .map(n => n.id),
              ),
            )
          }>
          Fold to sessions
        </button>
        <div className="spacer" />
        <button
          className="btn"
          onClick={() => setZoom(z => Math.max(0.4, z - 0.15))}
          aria-label="Zoom out">
          −
        </button>
        <span className="faint mono" style={{width: 44, textAlign: "center"}}>
          {Math.round(zoom * 100)}%
        </span>
        <button
          className="btn"
          onClick={() => setZoom(z => Math.min(1.8, z + 0.15))}
          aria-label="Zoom in">
          +
        </button>
      </div>
      <div
        style={{
          height: Math.min(height, svgHeight * zoom + 4),
          minHeight: 120,
          overflow: "auto",
          borderRadius: 12,
          border: "1px solid var(--line)",
          background:
            "radial-gradient(circle at 1px 1px, rgba(255,255,255,0.05) 1px, transparent 0) 0 0 / 22px 22px, var(--bg-2)",
          opacity: pending ? 0.55 : 1,
          transition: "opacity 0.2s",
        }}>
        <svg width={width * zoom} height={svgHeight * zoom} className="chart">
          <g transform={`scale(${zoom})`}>
            {layout.laid
              .links()
              .filter(l => l.source.data.node)
              .map(l => (
                <path
                  key={`${l.source.data.id}-${l.target.data.id}`}
                  className="tree-link"
                  d={link(l)}
                  style={{
                    stroke: l.target.data.node
                      ? STATES[l.target.data.node.state].color
                      : undefined,
                    strokeOpacity: 0.35,
                  }}
                />
              ))}
            {all.map(d => {
              const n = d.data.node!;
              const r = radius(n);
              const color = STATES[n.state].color;
              const hidden = folded.has(n.id) ? n.descendant_count : 0;
              const dim = !matches(n);
              const isSel = selected === n.id;
              const flashing = flash?.has(n.id);
              const line = nodeLine(n);
              const lx = r + 8;
              const ly = line ? -2 : 4;
              return (
                <g
                  key={n.id}
                  className={`tree-node ${flashing ? "flash" : ""}`}
                  style={{
                    transform: `translate(${d.y - COL + 30}px, ${d.x - layout.minX + ROW + TOP}px)`,
                    transition:
                      "transform 0.45s cubic-bezier(.2,.8,.2,1), opacity 0.3s",
                    opacity: dim ? 0.22 : 1,
                  }}>
                  {flashing ? (
                    <circle
                      r={r + 4}
                      fill="none"
                      stroke="white"
                      strokeWidth={2}
                      className="pulse"
                    />
                  ) : null}
                  {n.stale ? (
                    <circle
                      r={r + 5}
                      fill="none"
                      stroke={STALE_COLOR}
                      strokeWidth={1.8}
                      strokeDasharray="3 3"
                    />
                  ) : null}
                  {isSel ? (
                    <circle
                      r={r + 7}
                      fill="none"
                      stroke="white"
                      strokeWidth={1.5}
                      strokeOpacity={0.8}
                    />
                  ) : null}
                  <circle
                    className="body"
                    r={r}
                    fill={color}
                    fillOpacity={n.kind === "session" ? 0.9 : 0.25}
                    stroke={color}
                    strokeWidth={2}
                    onClick={() =>
                      n.child_count ? toggle(n.id) : onSelect?.(n)
                    }>
                    <title>
                      {n.child_count
                        ? `${folded.has(n.id) ? "Unfold" : "Fold"} the ${n.descendant_count} under it`
                        : nodeName(n)}
                    </title>
                  </circle>
                  {n.blocked ? (
                    <circle
                      cx={r * 0.8}
                      cy={-r * 0.8}
                      r={4}
                      fill={BLOCKED_COLOR}
                    />
                  ) : null}
                  <g onClick={() => onSelect?.(n)}>
                    <text
                      className="tree-label"
                      x={lx}
                      y={ly}
                      fontWeight={n.kind === "session" ? 650 : 450}>
                      {truncate(nodeName(n), NAME_CHARS)}
                      {hidden ? (
                        <tspan fill={color} fontWeight={650}>
                          {"  "}+{hidden}
                        </tspan>
                      ) : null}
                    </text>
                    {line ? (
                      <text className="tree-sub" x={lx} y={ly + 14}>
                        {truncate(line, SUB_CHARS)}
                      </text>
                    ) : null}
                  </g>
                </g>
              );
            })}
          </g>
        </svg>
      </div>
    </div>
  );
}

function truncate(s: string, n: number): string {
  return s.length > n ? `${s.slice(0, n - 1)}…` : s;
}
