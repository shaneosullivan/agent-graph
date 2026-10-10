"use client";

import * as d3 from "d3";

import {
  STATES,
  axisTime,
  duration,
  nodeName,
  shortTime,
  clock,
} from "@/app/showcase/_lib/format";
import {useWidth} from "@/app/showcase/_lib/useWidth";
import type {AgentNode} from "@/app/showcase/_lib/types";

import {useTooltip} from "./tooltip";

/** Events over time, as a histogram: when the graph was busy. */
export function ActivityBars({
  times,
  height = 120,
}: {
  times: Array<number>;
  height?: number;
}) {
  const {tip, show, hide} = useTooltip();
  const {ref, width} = useWidth();
  if (!times.length) {
    return <div ref={ref} />;
  }
  const m = {l: 30, r: 8, t: 8, b: 22};
  const extent = d3.extent(times) as [number, number];
  const x = d3
    .scaleTime()
    .domain(
      extent[0] === extent[1]
        ? [extent[0] - 60_000, extent[1] + 60_000]
        : extent,
    )
    .range([m.l, width - m.r]);
  const bins = d3
    .bin<number, Date>()
    .domain(x.domain() as [Date, Date])
    .thresholds(x.ticks(48))
    .value(t => new Date(t))(times);
  const y = d3
    .scaleLinear()
    .domain([0, d3.max(bins, b => b.length) ?? 1])
    .nice()
    .range([height - m.b, m.t]);

  return (
    <div ref={ref}>
      <svg width={width} height={height} className="chart">
        <defs>
          <linearGradient id="bars" x1="0" x2="0" y1="0" y2="1">
            <stop offset="0" stopColor="var(--accent-2)" />
            <stop offset="1" stopColor="var(--accent)" stopOpacity={0.4} />
          </linearGradient>
        </defs>
        {bins.map((b, i) => (
          <rect
            key={i}
            x={x(b.x0!) + 1}
            width={Math.max(1, x(b.x1!) - x(b.x0!) - 2)}
            y={y(b.length)}
            height={height - m.b - y(b.length)}
            rx={2}
            fill="url(#bars)"
            onMouseMove={e =>
              show(
                e,
                <>
                  <b>{b.length} events</b>
                  <div className="muted">
                    {clock(+b.x0!)} – {shortTime(+b.x1!)}
                  </div>
                </>,
              )
            }
            onMouseLeave={hide}
          />
        ))}
        {x.ticks(Math.max(2, Math.floor(width / 130))).map(t => (
          <text key={+t} x={x(t)} y={height - 6} textAnchor="middle">
            {axisTime(+t, +x.domain()[1] - +x.domain()[0])}
          </text>
        ))}
      </svg>
      {tip}
    </div>
  );
}

/**
 * Every node as a bar, from when it started to when it ended (or was last
 * heard from), in tree order: sessions, with what they started under them.
 */
export function Gantt({
  nodes,
  now,
  onPick,
  from,
}: {
  nodes: Array<AgentNode>;
  now: number;
  onPick?: (n: AgentNode) => void;
  /** Where the axis starts; bars that started before are drawn from it. */
  from?: number;
}) {
  const {tip, show, hide} = useTooltip();
  const {ref, width} = useWidth();
  const row = 18;
  // Names take a fixed column, or on a narrow screen (a phone) a share of
  // it, so the bars keep most of the width.
  const labelW = width < 560 ? Math.round(width * 0.38) : 210;
  const height = nodes.length * row + 28;
  const end = d3.max(nodes, n => n.ended ?? n.last_event) ?? now;
  const start = Math.min(
    end - 60_000,
    Math.max(from ?? -Infinity, d3.min(nodes, n => n.created) ?? now),
  );
  const x = d3
    .scaleTime()
    .domain([start, Math.max(end, start + 60_000)])
    .range([labelW, width - 10]);

  return (
    <div ref={ref}>
      <svg width={width} height={height} className="chart">
        {x.ticks(Math.max(2, Math.floor((width - labelW) / 130))).map(t => (
          <g key={+t}>
            <line
              className="grid-line"
              x1={x(t)}
              x2={x(t)}
              y1={0}
              y2={height - 20}
            />
            <text x={x(t)} y={height - 6} textAnchor="middle">
              {axisTime(+t, +x.domain()[1] - +x.domain()[0])}
            </text>
          </g>
        ))}
        {nodes.map((n, i) => {
          const y = i * row + 4;
          const to = n.ended ?? n.last_event;
          const color = STATES[n.state].color;
          return (
            <g
              key={n.id}
              style={{cursor: onPick ? "pointer" : undefined}}
              onClick={() => onPick?.(n)}
              onMouseMove={e =>
                show(
                  e,
                  <>
                    <b>{nodeName(n)}</b>
                    <div className="muted">
                      {STATES[n.state].label} · ran {duration(to - n.created)}
                      {n.ended ? "" : " so far"}
                    </div>
                    {n.headline ? (
                      <div style={{marginTop: 4}}>{n.headline}</div>
                    ) : null}
                  </>,
                )
              }
              onMouseLeave={hide}>
              <text
                x={8 + n.depth * 12}
                y={y + 11}
                style={{
                  fill: n.kind === "session" ? "var(--text)" : "var(--muted)",
                }}>
                {(n.kind === "session" ? "" : "↳ ") +
                  truncate(
                    nodeName(n),
                    Math.floor((labelW - 8 - n.depth * 12) / 7) - 2,
                  )}
              </text>
              <rect
                x={x(Math.max(start, n.created))}
                y={y + 2}
                width={Math.max(
                  3,
                  x(Math.max(start, to)) - x(Math.max(start, n.created)),
                )}
                height={row - 6}
                rx={4}
                fill={color}
                fillOpacity={n.kind === "session" ? 0.85 : 0.55}
              />
              {n.ended ? null : (
                // Still going: fades out to the right.
                <rect
                  x={x(Math.max(start, to)) - 1}
                  y={y + 2}
                  width={16}
                  height={row - 6}
                  rx={4}
                  fill={`url(#fade-${n.state})`}
                />
              )}
            </g>
          );
        })}
        <defs>
          {Object.entries(STATES).map(([k, s]) => (
            <linearGradient key={k} id={`fade-${k}`}>
              <stop offset="0" stopColor={s.color} stopOpacity={0.6} />
              <stop offset="1" stopColor={s.color} stopOpacity={0} />
            </linearGradient>
          ))}
        </defs>
      </svg>
      {tip}
    </div>
  );
}

/** How long each agent ran, a dot each, a row per agent type, with each type's median. */
export function DurationStrips({
  nodes,
  now,
}: {
  nodes: Array<AgentNode>;
  now: number;
}) {
  const {tip, show, hide} = useTooltip();
  const {ref, width} = useWidth();
  const agents = nodes.filter(n => n.kind === "agent");
  const byType = d3.group(agents, n => n.agent_type ?? "(untyped)");
  const types = [...byType.keys()]
    .sort((a, b) => byType.get(b)!.length - byType.get(a)!.length)
    .slice(0, 10);
  if (!types.length) {
    return (
      <div ref={ref} className="empty">
        No agents in this graph yet.
      </div>
    );
  }
  const row = 40;
  const labelW = 130;
  const height = types.length * row + 30;
  const all = agents.map(n => Math.max(1000, (n.ended ?? now) - n.created));
  const x = d3
    .scaleLog()
    .domain([
      Math.max(1000, d3.min(all) ?? 1000),
      Math.max(60_000, d3.max(all) ?? 60_000),
    ])
    .range([labelW, width - 20])
    .nice();

  return (
    <div ref={ref}>
      <svg width={width} height={height} className="chart">
        {x.ticks(Math.max(2, Math.floor((width - labelW) / 90))).map(t => (
          <g key={t}>
            <line
              className="grid-line"
              x1={x(t)}
              x2={x(t)}
              y1={0}
              y2={height - 20}
            />
            <text x={x(t)} y={height - 4} textAnchor="middle">
              {duration(t)}
            </text>
          </g>
        ))}
        {types.map((type, i) => {
          const list = byType.get(type)!;
          const y = i * row + row / 2;
          const med =
            d3.median(list, n =>
              Math.max(1000, (n.ended ?? now) - n.created),
            ) ?? 0;
          return (
            <g key={type}>
              <text
                x={8}
                y={y + 4}
                style={{fill: "var(--text)", fontSize: 12.5}}>
                {truncate(type, 18)}
              </text>
              <text x={8} y={y + 18} style={{fontSize: 10.5}}>
                {list.length} run{list.length === 1 ? "" : "s"}
              </text>
              <line
                x1={labelW}
                x2={width - 20}
                y1={y}
                y2={y}
                stroke="var(--line)"
              />
              <line
                x1={x(med)}
                x2={x(med)}
                y1={y - 13}
                y2={y + 13}
                stroke="white"
                strokeWidth={2}
                opacity={0.7}
              />
              {list.map((n, k) => {
                const d = Math.max(1000, (n.ended ?? now) - n.created);
                const jitter = ((k * 37) % 17) - 8;
                return (
                  <circle
                    key={n.id}
                    cx={x(d)}
                    cy={y + jitter}
                    r={5}
                    fill={STATES[n.state].color}
                    fillOpacity={0.8}
                    stroke="var(--bg)"
                    onMouseMove={e =>
                      show(
                        e,
                        <>
                          <b>{nodeName(n)}</b>
                          <div className="muted">
                            {duration(d)}
                            {n.ended ? "" : " so far"} · {STATES[n.state].label}
                          </div>
                          {n.purpose ? (
                            <div style={{marginTop: 4}}>{n.purpose}</div>
                          ) : null}
                        </>,
                      )
                    }
                    onMouseLeave={hide}
                  />
                );
              })}
            </g>
          );
        })}
      </svg>
      {tip}
    </div>
  );
}

/** How many nodes were running at once, over time. */
export function Concurrency({
  nodes,
  now,
  height = 160,
  from,
}: {
  nodes: Array<AgentNode>;
  now: number;
  height?: number;
  /** Where the axis starts. */
  from?: number;
}) {
  const {ref, width} = useWidth();
  const m = {l: 30, r: 10, t: 10, b: 22};
  const edges = nodes
    .flatMap(n => [
      {t: n.created, d: 1, agent: n.kind === "agent"},
      {
        t: n.ended ?? Math.min(now, n.last_event),
        d: -1,
        agent: n.kind === "agent",
      },
    ])
    .sort((a, b) => a.t - b.t || a.d - b.d);
  const every: Array<{t: number; all: number; agents: number}> = [];
  for (const e of edges) {
    const last = every.at(-1) ?? {all: 0, agents: 0};
    every.push({
      t: e.t,
      all: last.all + e.d,
      agents: last.agents + (e.agent ? e.d : 0),
    });
  }
  // From `from`: what was running then, and what changed after.
  const before = every.filter(p => from !== undefined && p.t < from).at(-1);
  const points = [
    ...(before ? [{...before, t: from!}] : []),
    ...every.filter(p => from === undefined || p.t >= from),
  ];
  if (points.length < 2) {
    return <div ref={ref} />;
  }
  const x = d3
    .scaleTime()
    .domain(d3.extent(points, p => p.t) as [number, number])
    .range([m.l, width - m.r]);
  const y = d3
    .scaleLinear()
    .domain([0, d3.max(points, p => p.all) ?? 1])
    .nice()
    .range([height - m.b, m.t]);
  const area = (key: "all" | "agents") =>
    d3
      .area<(typeof points)[number]>()
      .x(p => x(p.t))
      .y0(y(0))
      .y1(p => y(p[key]))
      .curve(d3.curveStepAfter)(points) ?? "";
  const peak = d3.max(points, p => p.all) ?? 0;
  const peakAt = points.find(p => p.all === peak)!;

  return (
    <div ref={ref}>
      <svg width={width} height={height} className="chart">
        {y.ticks(4).map(t => (
          <g key={t}>
            <line
              className="grid-line"
              x1={m.l}
              x2={width - m.r}
              y1={y(t)}
              y2={y(t)}
            />
            <text x={m.l - 6} y={y(t) + 4} textAnchor="end">
              {t}
            </text>
          </g>
        ))}
        <path
          d={area("all")}
          fill="var(--accent)"
          fillOpacity={0.25}
          stroke="var(--accent)"
        />
        <path
          d={area("agents")}
          fill="var(--accent-2)"
          fillOpacity={0.35}
          stroke="var(--accent-2)"
        />
        <circle cx={x(peakAt.t)} cy={y(peak)} r={4} fill="white" />
        <text x={x(peakAt.t) + 8} y={y(peak) + 4} style={{fill: "white"}}>
          peak: {peak} at once
        </text>
        {x.ticks(Math.max(2, Math.floor(width / 130))).map(t => (
          <text key={+t} x={x(t)} y={height - 6} textAnchor="middle">
            {axisTime(+t, +x.domain()[1] - +x.domain()[0])}
          </text>
        ))}
      </svg>
    </div>
  );
}

function truncate(s: string, n: number): string {
  return s.length > n ? `${s.slice(0, Math.max(1, n - 1))}…` : s;
}
