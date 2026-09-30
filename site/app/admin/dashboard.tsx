"use client";

import {useLayoutEffect, useRef, useState} from "react";

import type {Copied, Counter, Target} from "@/lib/analytics-core";
import {COPIED, TARGETS} from "@/lib/analytics-core";

/**
 * /admin's charts (app/admin/page.tsx gets the counts): the figures that
 * lead, one control for the range (the last 30 days, or 12 months) that
 * scopes everything under it, and a card per measure. Every chart shows
 * its values on hover and keyboard focus, and as a table; with nothing
 * counted in the range, it says so, and what would be counted.
 */

export type Period = {period: string; counts: Record<Counter, number>};

type Range = "days" | "months";

const MONTHS = "Jan Feb Mar Apr May Jun Jul Aug Sep Oct Nov Dec".split(" ");

/** A period, for people: "29 Sep", or "Sep 2026" (`short`: "Sep"). */
function periodLabel(period: string, short = false): string {
  const [y, m, d] = period.split("-");
  const month = MONTHS[Number(m) - 1];
  if (d) {
    return `${Number(d)} ${month}`;
  }
  return short ? month : `${month} ${y}`;
}

const number = (n: number) => n.toLocaleString("en-US");
const compact = (n: number) =>
  new Intl.NumberFormat("en-US", {
    notation: "compact",
    maximumFractionDigits: 1,
  }).format(n);

/** What each target's download button says (app/install.tsx). */
const TARGET_LABEL: Record<Target, string> = {
  "aarch64-apple-darwin": "macOS · Apple silicon",
  "x86_64-apple-darwin": "macOS · Intel",
  "aarch64-unknown-linux-musl": "Linux · ARM64",
  "x86_64-unknown-linux-musl": "Linux · x86_64",
  "aarch64-pc-windows-msvc": "Windows · ARM64",
  "x86_64-pc-windows-msvc": "Windows · x64",
};

/** What each copied command is (app/install.tsx): a copy's a download too. */
const COPIED_LABEL: Record<Copied, string> = {
  homebrew: "Homebrew command copied",
  "install-script": "Install script command copied",
  "claude-code": "Claude Code setup command copied",
  "claude-code-cloud": "Claude Code cloud setup command copied",
  codex: "Codex setup command copied",
};

/** Every kind of download: a button's target, or a command copied. */
const KINDS: Array<{counter: Counter; label: string}> = [
  ...TARGETS.map(t => ({
    counter: `download:${t}` as const,
    label: TARGET_LABEL[t],
  })),
  ...COPIED.map(c => ({
    counter: `download:copy:${c}` as const,
    label: COPIED_LABEL[c],
  })),
];

const downloads = (p: Period) =>
  KINDS.reduce((n, k) => n + p.counts[k.counter], 0);

export function Dashboard({
  days,
  months,
  active,
  disabled,
}: {
  days: Array<Period>;
  months: Array<Period>;
  active: number;
  disabled: boolean;
}) {
  const [range, setRange] = useState<Range>("days");
  const rows = range === "days" ? days : months;
  const span = range === "days" ? "the last 30 days" : "the last 12 months";
  const current = rows[rows.length - 1];
  const sum = (f: (p: Period) => number) => rows.reduce((n, p) => n + f(p), 0);
  const off = disabled
    ? "Counting is off (ANALYTICS_DISABLED is true), so nothing new is recorded."
    : null;

  return (
    <div className="admin">
      {off ? <p className="admin-off">{off}</p> : null}

      <div className="admin-tiles">
        <Tile
          label="Live shares now"
          value={active}
          note={active ? "sent events in the last 5 minutes" : "none active"}
          hero
        />
        <Tile
          label={range === "days" ? "Visitors today" : "Visitors this month"}
          value={current.counts.visitor}
        />
        <Tile
          label="Page views"
          value={sum(p => p.counts.pageview)}
          note={span}
        />
        <Tile
          label="Watch sessions"
          value={sum(p => p.counts.watch)}
          note={span}
        />
        <Tile label="Downloads" value={sum(downloads)} note={span} />
        <Tile label="Sign-ups" value={sum(p => p.counts.signup)} note={span} />
      </div>

      <div className="admin-range" role="radiogroup" aria-label="Range">
        {(
          [
            ["days", "Last 30 days"],
            ["months", "Last 12 months"],
          ] as const
        ).map(([key, label]) => (
          <button
            key={key}
            type="button"
            role="radio"
            aria-checked={range === key}
            className={range === key ? "on" : ""}
            onClick={() => setRange(key)}>
            {label}
          </button>
        ))}
      </div>

      <ChartCard
        title="Visitors and page views"
        note={`Per ${range === "days" ? "day" : "month"}, ${span}. A browser is a visitor once a ${range === "days" ? "day" : "month"}.`}
        rows={rows}
        series={[
          {name: "Page views", value: p => p.counts.pageview},
          {name: "Visitors", value: p => p.counts.visitor},
        ]}
        kind="lines"
        empty={{
          title: `No visits in ${span}`,
          body: "A page view is counted each time someone opens a page on the site, and a visitor once per browser.",
        }}
        off={off}
      />

      <ChartCard
        title="Watch sessions"
        note={`Runs of agent-graph watch-remote, ${span}: a new live share, or carrying on with the last one.`}
        rows={rows}
        series={[
          {name: "New shares", value: p => p.counts["watch.new"]},
          {
            name: "Carried on",
            value: p => p.counts.watch - p.counts["watch.new"],
          },
        ]}
        kind="stacked"
        empty={{
          title: `No watch sessions in ${span}`,
          body: "One's counted each time someone runs agent-graph watch-remote and it starts sharing.",
        }}
        off={off}
      />

      <ChartCard
        title="Downloads"
        note={`Download buttons clicked on the home page, ${span}.`}
        rows={rows}
        series={[{name: "Downloads", value: downloads}]}
        kind="stacked"
        empty={{
          title: `No downloads in ${span}`,
          body: "A download is counted when someone clicks a download button, or copies a command, in the install section of the home page.",
        }}
        off={off}
      />

      <TargetsCard rows={rows} span={span} off={off} />

      <ChartCard
        title="Sign-ups"
        note={`Accounts made, ${span}.`}
        rows={rows}
        series={[{name: "Sign-ups", value: p => p.counts.signup}]}
        kind="stacked"
        empty={{
          title: `No sign-ups in ${span}`,
          body: "An account is counted the first time it logs in, on the site or with agent-graph watch-remote.",
        }}
        off={off}
      />
    </div>
  );
}

function Tile({
  label,
  value,
  note,
  hero,
}: {
  label: string;
  value: number;
  note?: string;
  hero?: boolean;
}) {
  return (
    <div className={hero ? "admin-tile admin-tile-hero" : "admin-tile"}>
      <div className="admin-tile-label">{label}</div>
      <div className="admin-tile-value" title={number(value)}>
        {compact(value)}
      </div>
      {note ? <div className="admin-tile-note">{note}</div> : null}
    </div>
  );
}

type Series = {name: string; value: (p: Period) => number};

/** The width `ref`'s element is laid out at, as it changes. */
function useWidth() {
  const ref = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(0);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) {
      return;
    }
    const seen = new ResizeObserver(([entry]) =>
      setWidth(Math.floor(entry.contentRect.width)),
    );
    seen.observe(el);
    return () => seen.disconnect();
  }, []);
  return [ref, width] as const;
}

/** A clean top for an axis over `max`: 1, 2, 2.5 or 5 times a power of ten (4 at least). */
function niceMax(max: number): number {
  if (max <= 4) {
    return 4;
  }
  const p = 10 ** Math.floor(Math.log10(max));
  const m = max / p;
  return (m <= 1 ? 1 : m <= 2 ? 2 : m <= 2.5 ? 2.5 : m <= 5 ? 5 : 10) * p;
}

const PLOT_H = 180;
const AXIS_H = 26;
const LEFT = 44;
const RIGHT = 16;
const TOP = 10;

/**
 * A card with a chart of `series` over `rows`' periods: columns (stacked,
 * for more than one series), or lines. With every value 0, the empty state
 * instead, over the same axis.
 */
function ChartCard({
  title,
  note,
  rows,
  series,
  kind,
  empty,
  off,
}: {
  title: string;
  note: string;
  rows: Array<Period>;
  series: Array<Series>;
  kind: "stacked" | "lines";
  empty: {title: string; body: string};
  off: string | null;
}) {
  const [ref, width] = useWidth();
  const [at, setAt] = useState<number | null>(null);
  const values = rows.map(p => series.map(s => s.value(p)));
  const totals = values.map(v =>
    kind === "stacked" ? v.reduce((a, b) => a + b, 0) : Math.max(...v),
  );
  const isEmpty = totals.every(t => t === 0);
  const top = niceMax(Math.max(0, ...totals));
  const plotW = Math.max(0, width - LEFT - RIGHT);
  const slot = rows.length ? plotW / rows.length : 0;
  const x = (i: number) => LEFT + slot * (i + 0.5);
  const y = (v: number) => TOP + PLOT_H - (v / top) * PLOT_H;
  const barW = Math.max(2, Math.min(24, slot * 0.64));
  // Empty, only the baseline: nothing to read off, and nothing through the message.
  const ticks = isEmpty ? [0] : [0, top / 2, top];
  // About six dates along the axis, the last always.
  const every = Math.max(1, Math.ceil(rows.length / 6));
  const labelled = rows
    .map((_, i) => i)
    .filter(i => (rows.length - 1 - i) % every === 0);

  const step = (d: number) =>
    setAt(i =>
      Math.max(0, Math.min(rows.length - 1, (i ?? rows.length - 1) + d)),
    );

  return (
    <section className="card admin-chart" aria-label={title}>
      <div className="card-body">
        <h2>{title}</h2>
        <p className="admin-note">{note}</p>
        {series.length > 1 && !isEmpty ? (
          <ul className="admin-legend">
            {series.map((s, i) => (
              <li key={s.name}>
                <span
                  className={`key s${i + 1} ${kind === "lines" ? "line" : "box"}`}
                />
                {s.name}
              </li>
            ))}
          </ul>
        ) : null}
        <div className="admin-plot" ref={ref}>
          {width > 0 ? (
            <svg
              width={width}
              height={TOP + PLOT_H + AXIS_H}
              role="img"
              aria-label={
                isEmpty
                  ? empty.title
                  : `${title}: use the arrow keys to read each ${rows[0]?.period.length === 7 ? "month" : "day"}, or open the table below.`
              }
              tabIndex={isEmpty ? undefined : 0}
              onKeyDown={e => {
                if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
                  e.preventDefault();
                  step(e.key === "ArrowLeft" ? -1 : 1);
                }
              }}
              onFocus={() => setAt(i => i ?? rows.length - 1)}
              onBlur={() => setAt(null)}
              onPointerLeave={() => setAt(null)}>
              {ticks.map(t => (
                <g key={t}>
                  <line
                    className={t === 0 ? "admin-base" : "admin-grid"}
                    x1={LEFT}
                    x2={width - RIGHT}
                    y1={y(t)}
                    y2={y(t)}
                  />
                  {isEmpty ? null : (
                    <text
                      className="admin-tick"
                      x={LEFT - 8}
                      y={y(t) + 4}
                      textAnchor="end">
                      {compact(t)}
                    </text>
                  )}
                </g>
              ))}
              {labelled.map(i => (
                <text
                  key={rows[i].period}
                  className="admin-tick"
                  x={Math.min(Math.max(x(i), LEFT + 12), width - RIGHT - 12)}
                  y={TOP + PLOT_H + 18}
                  textAnchor="middle">
                  {periodLabel(rows[i].period, true)}
                </text>
              ))}

              {isEmpty
                ? null
                : kind === "stacked"
                  ? values.map((v, i) => {
                      let base = 0;
                      const last = v.reduce((l, n, s) => (n > 0 ? s : l), -1);
                      return (
                        <g
                          key={rows[i].period}
                          className={at === i ? "on" : ""}>
                          {v.map((n, s) => {
                            if (n <= 0) {
                              return null;
                            }
                            const y0 = y(base);
                            base += n;
                            // The 2px surface gap between stacked segments.
                            const y1 = y(base) + (s === last ? 0 : 2);
                            return (
                              <path
                                key={s}
                                className={`bar s${s + 1}`}
                                d={column(
                                  x(i) - barW / 2,
                                  y1,
                                  barW,
                                  y0 - y1,
                                  s === last,
                                )}
                              />
                            );
                          })}
                        </g>
                      );
                    })
                  : series.map((_, s) => (
                      <g key={s}>
                        <path
                          className={`line s${s + 1}`}
                          d={values
                            .map((v, i) => `${i ? "L" : "M"}${x(i)},${y(v[s])}`)
                            .join("")}
                        />
                        <circle
                          className={`dot s${s + 1}`}
                          cx={x(rows.length - 1)}
                          cy={y(values[values.length - 1][s])}
                          r={4}
                        />
                      </g>
                    ))}

              {!isEmpty && at !== null ? (
                <line
                  className="admin-cross"
                  x1={x(at)}
                  x2={x(at)}
                  y1={TOP}
                  y2={TOP + PLOT_H}
                />
              ) : null}
              {!isEmpty && at !== null && kind === "lines"
                ? series.map((_, s) => (
                    <circle
                      key={s}
                      className={`dot s${s + 1}`}
                      cx={x(at)}
                      cy={y(values[at][s])}
                      r={4}
                    />
                  ))
                : null}

              {/* Each period's hit area: its whole slot, not just the mark. */}
              {isEmpty
                ? null
                : rows.map((p, i) => (
                    <rect
                      key={p.period}
                      className="admin-hit"
                      x={LEFT + slot * i}
                      y={TOP}
                      width={slot}
                      height={PLOT_H}
                      onPointerMove={() => setAt(i)}
                    />
                  ))}
            </svg>
          ) : null}

          {isEmpty ? (
            <div className="admin-empty">
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <path d="M4 19h16M7 15v-3M12 15V8M17 15v-5" />
              </svg>
              <strong>{empty.title}</strong>
              <span>{off ?? empty.body}</span>
            </div>
          ) : null}

          {!isEmpty && at !== null && width > 0 ? (
            <div
              className="admin-tip"
              style={{
                left: Math.min(Math.max(x(at), 90), width - 90),
              }}>
              <div className="admin-tip-when">
                {periodLabel(rows[at].period)}
              </div>
              {series.map((s, i) => (
                <div key={s.name} className="admin-tip-row">
                  <span className={`key s${i + 1} line`} />
                  <strong>{number(values[at][i])}</strong>
                  <span>{s.name}</span>
                </div>
              ))}
            </div>
          ) : null}
        </div>

        {isEmpty ? null : (
          <details className="admin-table">
            <summary>Show as a table</summary>
            <table>
              <thead>
                <tr>
                  <th scope="col">
                    {rows[0]?.period.length === 7 ? "Month" : "Day"}
                  </th>
                  {series.map(s => (
                    <th key={s.name} scope="col">
                      {s.name}
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {[...rows].reverse().map((p, r) => (
                  <tr key={p.period}>
                    <th scope="row">{periodLabel(p.period)}</th>
                    {values[rows.length - 1 - r].map((n, i) => (
                      <td key={i}>{number(n)}</td>
                    ))}
                  </tr>
                ))}
              </tbody>
            </table>
          </details>
        )}
      </div>
    </section>
  );
}

/** A column's path: square at the baseline, its top rounded 4px if it's the stack's top. */
function column(x: number, y: number, w: number, h: number, round: boolean) {
  if (h <= 0) {
    return "";
  }
  const r = round ? Math.min(4, w / 2, h) : 0;
  return `M${x},${y + h}V${y + r}Q${x},${y} ${x + r},${y}H${x + w - r}Q${x + w},${y} ${x + w},${y + r}V${y + h}Z`;
}

/**
 * Downloads by kind over the range (each button's target, and each command
 * copied): a bar each, most first, its value at its tip.
 */
function TargetsCard({
  rows,
  span,
  off,
}: {
  rows: Array<Period>;
  span: string;
  off: string | null;
}) {
  const totals = KINDS.map(k => ({
    ...k,
    n: rows.reduce((n, p) => n + p.counts[k.counter], 0),
  })).sort((a, b) => b.n - a.n);
  const max = Math.max(0, ...totals.map(t => t.n));
  return (
    <section className="card admin-chart" aria-label="Downloads by kind">
      <div className="card-body">
        <h2>Downloads by kind</h2>
        <p className="admin-note">
          Which program was downloaded, or which command copied, {span}.
        </p>
        {max === 0 ? (
          <div className="admin-empty admin-empty-inline">
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path d="M4 7h10M4 12h6M4 17h13" />
            </svg>
            <strong>No downloads in {span}</strong>
            <span>
              {off ??
                "Each download button's clicks, and each install command's copies, show here."}
            </span>
          </div>
        ) : (
          <ul className="admin-bars">
            {totals.map(({counter, label, n}) => (
              <li key={counter} title={`${label}: ${number(n)}`}>
                <span className="admin-bar-label">{label}</span>
                <span className="admin-bar-track">
                  {n > 0 ? (
                    <span
                      className="admin-bar s1"
                      style={{width: `${(n / max) * 100}%`}}
                    />
                  ) : null}
                  <span className="admin-bar-value">{number(n)}</span>
                </span>
              </li>
            ))}
          </ul>
        )}
      </div>
    </section>
  );
}
