"use client";

import * as d3 from "d3";
import {useState} from "react";

export type Slice = {key: string; label: string; value: number; color: string};

/** A ring of slices, with the total (or the slice under the pointer) in the middle. */
export function Donut({
  slices,
  size = 220,
  centre = "nodes",
}: {
  slices: Array<Slice>;
  size?: number;
  centre?: string;
}) {
  const [hover, setHover] = useState<string | null>(null);
  const total = slices.reduce((n, s) => n + s.value, 0);
  const pie = d3
    .pie<Slice>()
    .value(s => s.value)
    .sort(null)
    .padAngle(0.025);
  const r = size / 2;
  const arc = (a: d3.PieArcDatum<Slice>, inner: number, outer: number) =>
    d3
      .arc<d3.PieArcDatum<Slice>>()
      .innerRadius(inner)
      .outerRadius(outer)
      .cornerRadius(4)(a);
  const shown = slices.find(s => s.key === hover);

  return (
    <svg
      width={size}
      height={size}
      viewBox={`${-r} ${-r} ${size} ${size}`}
      className="chart">
      {pie(slices.filter(s => s.value > 0)).map(a => {
        const on = hover === a.data.key;
        return (
          <path
            key={a.data.key}
            d={arc(a, r * 0.64, on ? r - 2 : r - 10) ?? undefined}
            fill={a.data.color}
            opacity={hover && !on ? 0.35 : 1}
            style={{
              transition: "opacity 0.2s",
              filter: on ? `drop-shadow(0 0 10px ${a.data.color})` : undefined,
            }}
            onMouseEnter={() => setHover(a.data.key)}
            onMouseLeave={() => setHover(null)}
          />
        );
      })}
      <text
        textAnchor="middle"
        y={-4}
        style={{
          fontSize: 34,
          fontWeight: 650,
          fill: shown?.color ?? "var(--text)",
        }}>
        {(shown?.value ?? total).toLocaleString()}
      </text>
      <text
        textAnchor="middle"
        y={20}
        style={{fontSize: 12, fill: "var(--muted)"}}>
        {shown ? shown.label.toLowerCase() : centre}
      </text>
    </svg>
  );
}
