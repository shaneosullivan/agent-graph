"use client";

import * as d3 from "d3";
import {useMemo, useState} from "react";

import {
  Concurrency,
  DurationStrips,
  Gantt,
} from "@/app/showcase/_components/charts/Timelines";
import {NodeDrawer} from "@/app/showcase/_components/NodeDrawer";
import {
  ErrorBox,
  Explainer,
  Legend,
  Loading,
  Tile,
} from "@/app/showcase/_components/ui";
import {GraphPage} from "@/app/showcase/_lib/graph";
import {usePhone} from "@/app/showcase/_lib/useWidth";
import {useApi, useLoad} from "@/app/showcase/_lib/client";
import {
  now as clockNow,
  STATES,
  duration,
  median,
  nodeName,
  plural,
} from "@/app/showcase/_lib/format";
import type {AgentNode} from "@/app/showcase/_lib/types";

const GANTT_ROWS = 80;

const HOUR = 60 * 60 * 1000;
const WINDOWS = [
  {label: "Last hour", ms: HOUR},
  {label: "Last 6 hours", ms: 6 * HOUR},
  {label: "Last day", ms: 24 * HOUR},
  {label: "Everything", ms: Infinity},
];

/** The shortest window, back from the last event, that holds most of the nodes. */
function bestWindow(nodes: Array<AgentNode>): number {
  const last = Math.max(...nodes.map(n => n.ended ?? n.last_event));
  for (const w of WINDOWS) {
    if (
      nodes.filter(n => (n.ended ?? n.last_event) >= last - w.ms).length >=
      nodes.length * 0.8
    ) {
      return w.ms;
    }
  }
  return Infinity;
}

export default function Page() {
  return <GraphPage>{graph => <Performance graph={graph} />}</GraphPage>;
}

function Performance({graph}: {graph: string}) {
  const {getAll} = useApi();
  const phone = usePhone();
  const [selected, setSelected] = useState<string | null>(null);
  const [rows, setRows] = useState(GANTT_ROWS);
  const [windowMs, setWindowMs] = useState<number | null>(null);
  const {data, error, loading} = useLoad(
    async () =>
      (await getAll<AgentNode>(`/graphs/${graph}/nodes`, {order: "tree"})).data,
    [graph],
  );
  const now = clockNow();
  const span = windowMs ?? (data?.length ? bestWindow(data) : Infinity);
  const last = data?.length
    ? Math.max(...data.map(n => n.ended ?? n.last_event))
    : now;
  const from = span === Infinity ? undefined : last - span;

  const stats = useMemo(() => {
    if (!data) {
      return null;
    }
    const agents = data.filter(n => n.kind === "agent");
    const finished = agents.filter(n => n.ended);
    const runs = finished.map(n => n.ended! - n.created);
    const failed = agents.filter(n => n.state === "failed").length;
    const byType = d3.rollups(
      finished,
      list => ({
        n: list.length,
        median: d3.median(list, n => n.ended! - n.created) ?? 0,
      }),
      n => n.agent_type ?? "(untyped)",
    );
    const slowest = byType
      .filter(([, v]) => v.n >= 2)
      .sort((a, b) => b[1].median - a[1].median)[0];
    const fanout = [...data]
      .filter(n => n.child_count > 0)
      .sort((a, b) => b.child_count - a.child_count);
    const sessions = data.filter(n => n.kind === "session");
    const perSession = sessions.length ? agents.length / sessions.length : 0;
    const longest = [...agents].sort(
      (a, b) => (b.ended ?? now) - b.created - ((a.ended ?? now) - a.created),
    )[0];
    // Peak concurrency.
    const edges = data
      .flatMap(n => [
        {t: n.created, d: 1},
        {t: n.ended ?? Math.min(now, n.last_event), d: -1},
      ])
      .sort((a, b) => a.t - b.t || a.d - b.d);
    let open = 0;
    let peak = 0;
    for (const e of edges) {
      peak = Math.max(peak, (open += e.d));
    }
    return {
      agents,
      runs,
      failed,
      slowest,
      fanout,
      perSession,
      longest,
      peak,
      byType,
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [data]);

  return (
    <>
      <Explainer
        kicker="Performance"
        title="Where the time goes"
        lede="When each agent started and stopped, what kind it was, and how it ended: enough to see which work is slow, what runs in parallel, and where the failures cluster."
        what={
          <>
            <p>
              Every node, from <b>one list</b> in tree order (
              <code>order=tree</code>, up to 1,000 a page), each with when it
              was <code>created</code>, when it <code>ended</code>, its{" "}
              <code>agent_type</code> and its state. Everything here is worked
              out from that, in the browser.
            </p>
            <p>Click a bar for the node in full.</p>
          </>
        }
        why={
          <p>
            Tuning agents is guesswork without this. Which agent types take
            longest, and whether they run in parallel or one after another;
            which sessions fan out into dozens of helpers; whether failures are
            one type&rsquo;s problem or everyone&rsquo;s. Feed it to a
            dashboard, or to a model that suggests changes.
          </p>
        }
        calls={["GET /graphs/{id}/nodes?order=tree&limit=1000"]}
      />
      {loading ? <Loading what="Fetching every node" /> : null}
      {error ? <ErrorBox error={error} /> : null}
      {data && stats ? (
        <div className="grid side">
          <div className="stack">
            <div className="tiles">
              <Tile
                label="Median agent run"
                value={stats.runs.length ? duration(median(stats.runs)) : "–"}
                note={`${stats.runs.length} finished`}
              />
              <Tile
                label="Longest agent"
                value={
                  stats.longest
                    ? duration(
                        (stats.longest.ended ?? now) - stats.longest.created,
                      )
                    : "–"
                }
                note={stats.longest ? nodeName(stats.longest) : undefined}
              />
              <Tile
                label="Peak at once"
                value={stats.peak}
                note="nodes running together"
                color="var(--accent-2)"
              />
              <Tile
                label="Agents per session"
                value={stats.perSession.toFixed(1)}
              />
              <Tile
                label="Failed"
                value={
                  stats.agents.length
                    ? `${Math.round((100 * stats.failed) / stats.agents.length)}%`
                    : "–"
                }
                note={plural(stats.failed, "agent")}
                color={STATES.failed.color}
              />
            </div>

            <div className="panel">
              <h2>Insights</h2>
              <ul style={{margin: 0, paddingLeft: 20, lineHeight: 1.8}}>
                {stats.slowest ? (
                  <li>
                    <b>{stats.slowest[0]}</b> agents are the slowest kind: a
                    median of <b>{duration(stats.slowest[1].median)}</b> over{" "}
                    {stats.slowest[1].n} runs.
                  </li>
                ) : null}
                {stats.fanout[0] ? (
                  <li>
                    <b>{nodeName(stats.fanout[0])}</b> started the most work:{" "}
                    {plural(stats.fanout[0].child_count, "helper")} directly,{" "}
                    {stats.fanout[0].descendant_count} in all.
                  </li>
                ) : null}
                <li>
                  At the busiest moment, <b>{stats.peak}</b> nodes were running
                  at once.
                </li>
                {stats.failed ? (
                  <li>
                    {plural(stats.failed, "agent")} failed:{" "}
                    {d3
                      .rollups(
                        stats.agents.filter(n => n.state === "failed"),
                        l => l.length,
                        n => n.agent_type ?? "(untyped)",
                      )
                      .map(([t, k]) => `${k} ${t}`)
                      .join(", ")}
                    .
                  </li>
                ) : (
                  <li>No agent failed.</li>
                )}
              </ul>
            </div>

            {phone ? (
              <label className="picker">
                <span className="faint">Time window</span>
                <select
                  value={WINDOWS.findIndex(w => w.ms === span)}
                  onChange={e => setWindowMs(WINDOWS[+e.target.value].ms)}>
                  {WINDOWS.map((w, i) => (
                    <option key={w.label} value={i}>
                      {w.label}
                    </option>
                  ))}
                </select>
              </label>
            ) : (
              <div className="row" style={{gap: 8}}>
                <span className="faint" style={{fontSize: 13}}>
                  Time window:
                </span>
                {WINDOWS.map(w => (
                  <button
                    key={w.label}
                    className={`chip ${span === w.ms ? "on" : ""}`}
                    onClick={() => setWindowMs(w.ms)}>
                    {w.label}
                  </button>
                ))}
              </div>
            )}

            <div className="panel">
              <h2>Timeline</h2>
              <p className="sub">
                Each node, from when it started to when it ended; a bar that
                fades out is still going. Sessions, with what they started under
                them.
              </p>
              <Gantt
                nodes={data.slice(0, rows)}
                now={now}
                from={from}
                onPick={n => setSelected(n.id)}
              />
              {data.length > rows ? (
                <button
                  className="btn"
                  style={{marginTop: 10}}
                  onClick={() => setRows(r => r + GANTT_ROWS)}>
                  Show {Math.min(GANTT_ROWS, data.length - rows)} more of{" "}
                  {data.length}
                </button>
              ) : null}
              <div style={{marginTop: 10}}>
                <Legend />
              </div>
            </div>

            <div className="grid two">
              <div className="panel">
                <h2>How long each kind of agent takes</h2>
                <p className="sub">
                  A dot per agent (log scale); the white tick is each
                  type&rsquo;s median.
                </p>
                <DurationStrips nodes={data} now={now} />
              </div>
              <div className="panel">
                <h2>Running at once</h2>
                <p className="sub">
                  <span style={{color: "var(--accent)"}}>■</span> every node{" "}
                  <span style={{color: "var(--accent-2)", marginLeft: 8}}>
                    ■
                  </span>{" "}
                  agents
                </p>
                <Concurrency nodes={data} now={now} from={from} height={220} />
              </div>
            </div>

            <div className="panel">
              <h2>Who starts the most work</h2>
              <p className="sub">
                Nodes by how many helpers they started directly (and how many
                are under them in all).
              </p>
              {stats.fanout.slice(0, 8).map(n => (
                <div
                  key={n.id}
                  style={{marginBottom: 10, cursor: "pointer"}}
                  onClick={() => setSelected(n.id)}>
                  <div className="row" style={{fontSize: 14}}>
                    <b>{nodeName(n)}</b>
                    <span className="faint">{n.kind}</span>
                    <div className="spacer" />
                    <span className="mono muted">
                      {n.child_count} direct · {n.descendant_count} in all
                    </span>
                  </div>
                  <div className="progress" style={{marginTop: 6}}>
                    <div
                      style={{
                        width: `${(100 * n.descendant_count) / Math.max(1, stats.fanout[0].descendant_count)}%`,
                      }}
                    />
                  </div>
                </div>
              ))}
            </div>
          </div>
          {selected ? (
            <NodeDrawer
              graph={graph}
              nodeId={selected}
              onSelect={setSelected}
              onClose={() => setSelected(null)}
            />
          ) : (
            <div className="panel muted" style={{alignSelf: "start"}}>
              Click a bar or a dot to see that node in full.
            </div>
          )}
        </div>
      ) : null}
    </>
  );
}
