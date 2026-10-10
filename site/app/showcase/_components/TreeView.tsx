"use client";

import * as d3 from "d3";
import {useMemo, useState} from "react";

import {
  BLOCKED_COLOR,
  STALE_COLOR,
  STATES,
  nodeLine,
  nodeName,
} from "@/app/showcase/_lib/format";
import {usePhone, useWidth} from "@/app/showcase/_lib/useWidth";
import type {AgentNode} from "@/app/showcase/_lib/types";

type Item = {node: AgentNode | null; id: string; kids: Array<Item>};

const ROW = 34;
const COL = 250;
/** Room above the first node, for a label drawn above it. */
const TOP = 16;
/** How much of a name, and of what it's doing, a label shows. */
const NAME_CHARS = 28;
const SUB_CHARS = 32;

/** Roughly how wide a character of a name, and of what it's doing, is drawn. */
const NAME_PX = 6.9;
const SUB_PX = 6.1;
/** A line's height, in a name and in what it's doing. */
const NAME_LH = 14;
const SUB_LH = 13;

/** A label's lines: its name's, and what it's doing's. */
type Label = {name: Array<string>; sub: Array<string>};

/**
 * `text` in at most `lines` lines of at most `chars` characters, broken
 * between words where it can, and cut short with "…" if it doesn't fit.
 */
function wrap(text: string, chars: number, lines: number): Array<string> {
  const out: Array<string> = [];
  let rest = text.trim();
  while (rest && out.length < lines) {
    if (rest.length <= chars) {
      out.push(rest);
      rest = "";
      break;
    }
    if (out.length === lines - 1) {
      out.push(truncate(rest, chars));
      rest = "";
      break;
    }
    const space = rest.lastIndexOf(" ", chars);
    const cut = space > chars / 3 ? space : chars;
    out.push(rest.slice(0, cut).trimEnd());
    rest = rest.slice(cut).trimStart();
  }
  return out;
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
  const phone = usePhone();
  const {ref: boxRef, width: boxWidth} = useWidth();

  // On a phone, a session's name and what it's doing each wrap to two
  // lines, in at most 30% of the box's width, so the tree is narrower;
  // every other label is a line of each, as on a wider screen.
  const wrapPx = Math.max(84, Math.round(boxWidth * 0.3));
  const labelOf = (n: AgentNode): Label => {
    const line = nodeLine(n);
    if (phone && n.kind === "session") {
      return {
        name: wrap(nodeName(n), Math.floor(wrapPx / NAME_PX), 2),
        sub: line ? wrap(line, Math.floor(wrapPx / SUB_PX), 2) : [],
      };
    }
    return {
      name: [truncate(nodeName(n), NAME_CHARS)],
      sub: line ? [truncate(line, SUB_CHARS)] : [],
    };
  };
  /** How tall a node's label is. */
  const labelHeight = (n: AgentNode) => {
    const l = labelOf(n);
    return l.name.length * NAME_LH + l.sub.length * SUB_LH;
  };
  /** Where a node's label ends, roughly (so lines leave from there, not through it). */
  const labelEnd = (n: AgentNode): number => {
    const l = labelOf(n);
    const chars = Math.max(
      ...l.name.map(t => t.length * NAME_PX),
      ...l.sub.map(t => t.length * SUB_PX),
    );
    const end = radius(n) + 14 + chars;
    return phone ? end : Math.min(COL - 30, end);
  };

  // Start folded below a depth, each time a different tree arrives.
  // (Set as it renders, not in an effect: React's way to reset state when
  // a prop changes.)
  const rootKey = `${forest.kids.map(k => k.id).join(",")}/${foldBelow}`;
  const [foldedFor, setFoldedFor] = useState<string | null>(null);
  if (foldBelow !== undefined && foldedFor !== rootKey) {
    setFoldedFor(rootKey);
    const f = new Set<string>();
    for (const n of nodes) {
      if (n.depth >= foldBelow && n.child_count > 0) {
        f.add(n.id);
      }
    }
    setFolded(f);
  }

  const layout = useMemo(() => {
    const root = d3.hierarchy<Item>(forest, d =>
      d.id !== "__forest" && folded.has(d.id) ? undefined : d.kids,
    );
    const tree = d3
      .tree<Item>()
      .nodeSize([ROW, COL])
      .separation((a, b) => {
        const base = a.parent === b.parent ? 1 : 1.25;
        if (!phone || !a.data.node || !b.data.node) {
          return base;
        }
        // Room for two wrapped labels, one above the other.
        const need =
          (labelHeight(a.data.node) + labelHeight(b.data.node)) / 2 + 8;
        return Math.max(base, need / ROW);
      });
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
    // labelHeight changes only with these.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [forest, folded, phone, wrapPx]);

  const q = query.trim().toLowerCase();
  const matches = (n: AgentNode) =>
    !q ||
    `${nodeName(n)} ${nodeLine(n)} ${n.agent_type ?? ""} ${n.state}`
      .toLowerCase()
      .includes(q);

  // Each level's column: COL wide; or on a phone, as wide as its widest
  // label needs, and the curve to the next level.
  const depths = layout.laid.height;
  const columnOf = (depth: number) => {
    if (!phone) {
      return COL;
    }
    const ends = layout.laid
      .descendants()
      .filter(d => d.depth === depth && d.data.node)
      .map(d => labelEnd(d.data.node!) + (folded.has(d.data.id) ? 30 : 0));
    return Math.max(120, ...ends) + 48;
  };
  const xs = [0, 30];
  for (let depth = 1; depth < depths; depth++) {
    xs.push(xs[depth] + columnOf(depth));
  }
  const width = phone
    ? xs[depths] + columnOf(depths) - 20
    : layout.maxY + COL + 40;
  const svgHeight = layout.maxX - layout.minX + ROW * 2 + TOP;
  const at = (d: d3.HierarchyPointNode<Item>): [number, number] => [
    xs[d.depth],
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
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      return next;
    });

  const all = layout.laid.descendants().filter(d => d.data.node);
  // Folded from depth `d` down: what's shown is `d` levels under each
  // session. (0: nothing folded.)
  const foldedFrom = (d: number) =>
    new Set(
      d
        ? nodes.filter(n => n.depth >= d && n.child_count > 0).map(n => n.id)
        : [],
    );
  const unfoldAll = () => setFolded(new Set());
  const foldToSessions = () => setFolded(foldedFrom(1));
  // How deep it's shown now, if it's one of those (else it's been folded
  // by hand): what the phone's Depth menu says.
  const sameAs = (a: Set<string>) =>
    a.size === folded.size && [...a].every(id => folded.has(id));
  const depth = [0, 1, 2].find(d => sameAs(foldedFrom(d))) ?? "custom";

  return (
    <div>
      <div className="row tree-tools" style={{marginBottom: 10}}>
        {phone ? (
          // On a phone, how deep the tree's shown is a menu, beside the zoom.
          <label className="picker">
            <span className="faint">Depth</span>
            <select
              value={String(depth)}
              onChange={e => setFolded(foldedFrom(Number(e.target.value)))}>
              <option value="0">Every level</option>
              <option value="1">Sessions + 1 level</option>
              <option value="2">Sessions + 2 levels</option>
              {depth === "custom" ? (
                <option value="custom" disabled>
                  As you&rsquo;ve folded it
                </option>
              ) : null}
            </select>
          </label>
        ) : (
          <>
            <button className="btn" onClick={unfoldAll}>
              Unfold all
            </button>
            <button className="btn" onClick={foldToSessions}>
              Fold to sessions
            </button>
          </>
        )}
        <div className="spacer" />
        <div className="zoom">
          <button
            className="btn"
            onClick={() => setZoom(z => Math.max(0.4, z - 0.15))}
            disabled={zoom <= 0.4}
            aria-label="Zoom out"
            title="Zoom out">
            <Magnifier plus={false} />
          </button>
          <button
            className="btn"
            onClick={() => setZoom(z => Math.min(1.8, z + 0.15))}
            disabled={zoom >= 1.8}
            aria-label="Zoom in"
            title="Zoom in">
            <Magnifier plus />
          </button>
        </div>
      </div>
      <div
        ref={boxRef}
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
        {/* Its own width, scrolled sideways in this box: not squeezed to it. */}
        <svg
          width={width * zoom}
          height={svgHeight * zoom}
          className="chart"
          style={{maxWidth: "none"}}>
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
              const label = labelOf(n);
              const lx = r + 8;
              // The label's lines, centred on the dot.
              const ly = -labelHeight(n) / 2 + 10;
              return (
                <g
                  key={n.id}
                  className={`tree-node ${flashing ? "flash" : ""}`}
                  style={{
                    transform: `translate(${at(d)[0]}px, ${at(d)[1]}px)`,
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
                      {label.name.map((t, i) => (
                        <tspan key={i} x={lx} dy={i ? NAME_LH : 0}>
                          {t}
                          {hidden && i === label.name.length - 1 ? (
                            <tspan fill={color} fontWeight={650}>
                              {"  "}+{hidden}
                            </tspan>
                          ) : null}
                        </tspan>
                      ))}
                    </text>
                    {label.sub.length ? (
                      <text
                        className="tree-sub"
                        x={lx}
                        y={ly + label.name.length * NAME_LH}>
                        {label.sub.map((t, i) => (
                          <tspan key={i} x={lx} dy={i ? SUB_LH : 0}>
                            {t}
                          </tspan>
                        ))}
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

/** A magnifying glass, with a minus or a plus in it. */
function Magnifier({plus}: {plus: boolean}) {
  return (
    <svg viewBox="0 0 24 24" aria-hidden>
      <circle cx="10.5" cy="10.5" r="6.5" />
      <path d="M15.5 15.5 20 20M7.5 10.5h6" />
      {plus ? <path d="M10.5 7.5v6" /> : null}
    </svg>
  );
}

function truncate(s: string, n: number): string {
  return s.length > n ? `${s.slice(0, n - 1)}…` : s;
}
