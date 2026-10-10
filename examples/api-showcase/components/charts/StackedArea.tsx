"use client";

import * as d3 from "d3";
import {useMemo, useRef} from "react";

import {STATE_ORDER, STATES, axisTime} from "@/lib/format";
import {useWidth} from "@/lib/useWidth";
import type {NodeState} from "@/lib/types";

export type Point = {t: number; counts: Record<NodeState, number>};

/**
 * How many nodes were in each state after every event, stacked: the
 * graph's whole history in one picture. Click (or drag) to go to a moment.
 */
export function StackedArea({
  points,
  at,
  onPick,
  height = 230,
}: {
  points: Array<Point>;
  at: number;
  onPick: (i: number) => void;
  height?: number;
}) {
  const ref = useRef<SVGSVGElement>(null);
  const {ref: box, width} = useWidth();
  const m = {l: 34, r: 12, t: 10, b: 24};
  const {x, y, areas, ticks} = useMemo(() => {
    const x = d3
      .scaleLinear()
      .domain([0, Math.max(1, points.length - 1)])
      .range([m.l, width - m.r]);
    const stack = d3
      .stack<Point, NodeState>()
      .keys(STATE_ORDER)
      .value((p, k) => p.counts[k] ?? 0);
    const series = stack(points);
    const max = d3.max(series.at(-1) ?? [], d => d[1]) ?? 1;
    const y = d3
      .scaleLinear()
      .domain([0, Math.max(1, max)])
      .nice()
      .range([height - m.b, m.t]);
    const area = d3
      .area<d3.SeriesPoint<Point>>()
      .x((_, i) => x(i))
      .y0(d => y(d[0]))
      .y1(d => y(d[1]))
      .curve(d3.curveMonotoneX);
    const areas = series.map(s => ({key: s.key, d: area(s) ?? ""}));
    const n = Math.min(Math.max(2, Math.floor(width / 140)), points.length);
    const ticks = Array.from({length: n}, (_, k) =>
      Math.round((k * (points.length - 1)) / Math.max(1, n - 1)),
    );
    return {x, y, areas, ticks};
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [points, height, width]);

  const pick = (e: React.PointerEvent) => {
    if (e.type === "pointermove" && e.buttons !== 1) return;
    const box = ref.current!.getBoundingClientRect();
    const px = ((e.clientX - box.left) / box.width) * width;
    onPick(Math.max(0, Math.min(points.length - 1, Math.round(x.invert(px)))));
  };

  return (
    <div ref={box}>
      <svg
        ref={ref}
        width={width}
        height={height}
        className="chart"
        style={{cursor: "crosshair", touchAction: "none"}}
        onPointerDown={pick}
        onPointerMove={pick}>
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
        {areas.map(a => (
          <path
            key={a.key}
            d={a.d}
            fill={STATES[a.key as NodeState].color}
            fillOpacity={0.78}
          />
        ))}
        {ticks.map(i =>
          points[i] ? (
            <text key={i} x={x(i)} y={height - 6} textAnchor="middle">
              {axisTime(
                points[i].t,
                (points.at(-1)?.t ?? 0) - (points[0]?.t ?? 0),
              )}
            </text>
          ) : null,
        )}
        <line
          x1={x(at)}
          x2={x(at)}
          y1={m.t - 6}
          y2={height - m.b}
          stroke="white"
          strokeWidth={2}
        />
        <circle cx={x(at)} cy={m.t - 4} r={5} fill="white" />
      </svg>
    </div>
  );
}
