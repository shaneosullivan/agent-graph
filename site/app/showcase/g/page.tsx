"use client";

import {useRouter} from "next/navigation";

import {Donut} from "@/app/showcase/_components/charts/Donut";
import {ActivityBars} from "@/app/showcase/_components/charts/Timelines";
import {
  ErrorBox,
  Explainer,
  Loading,
  NodeRow,
  Tile,
} from "@/app/showcase/_components/ui";
import {GraphPage, graphHref} from "@/app/showcase/_lib/graph";
import {useApi, useLoad} from "@/app/showcase/_lib/client";
import {
  STATE_ORDER,
  STATES,
  ago,
  clock,
  duration,
  plural,
} from "@/app/showcase/_lib/format";
import type {
  AgentEvent,
  AgentNode,
  Graph,
  List,
} from "@/app/showcase/_lib/types";

export default function Page() {
  return <GraphPage>{graph => <Overview graph={graph} />}</GraphPage>;
}

function Overview({graph}: {graph: string}) {
  const router = useRouter();
  const {get, getAll} = useApi();
  const {data, error, loading} = useLoad(async () => {
    const [g, roots, events] = await Promise.all([
      get<Graph>(`/graphs/${graph}`),
      get<List<AgentNode>>(`/graphs/${graph}/nodes`, {
        is_root: true,
        limit: 100,
      }),
      getAll<AgentEvent>(`/graphs/${graph}/events`, {limit: 1000}, 3),
    ]);
    return {graph: g.data!, roots: roots.data!.data, events: events.data};
  }, [graph]);

  return (
    <>
      <Explainer
        kicker="Overview"
        title="Where everything stands"
        lede="The graph at a glance: how many sessions and agents there are, what state they're in, and when they were busy."
        what={
          <p>
            A graph is one of your live shares. Its <b>counts</b> come with the
            graph itself, in one small call, however many nodes it has. The
            histogram is every event it holds, by when it happened; the list is
            each tree&rsquo;s root: the sessions you started.
          </p>
        }
        why={
          <p>
            It&rsquo;s the cheapest health check there is: one request says
            whether anything needs a person, is stuck behind something, or has
            gone quiet. Poll it with <code>If-None-Match</code> and an unchanged
            graph costs nothing at all (a <code>304</code>).
          </p>
        }
        calls={[
          "GET /graphs/{graph_id}",
          "GET /graphs/{graph_id}/nodes?is_root=true",
          "GET /graphs/{graph_id}/events",
        ]}
      />
      {loading ? <Loading /> : null}
      {error ? <ErrorBox error={error} /> : null}
      {data ? (
        <Body
          {...data}
          onRoot={id => router.push(graphHref(graph, "explorer", {root: id}))}
        />
      ) : null}
    </>
  );
}

function Body({
  graph,
  roots,
  events,
  onRoot,
}: {
  graph: Graph;
  roots: Array<AgentNode>;
  events: Array<AgentEvent>;
  onRoot: (id: string) => void;
}) {
  const c = graph.counts;
  const r = graph.retention;
  const span =
    r.first_event_created && r.last_event_created
      ? r.last_event_created - r.first_event_created
      : 0;
  return (
    <div className="stack">
      <div className="tiles">
        <Tile
          label="Nodes"
          value={c.nodes}
          note={`${c.sessions} sessions · ${c.agents} agents`}
        />
        <Tile
          label="Working now"
          value={c.by_state.working}
          color={STATES.working.color}
        />
        <Tile
          label="Need you"
          value={c.by_state.input_required}
          color={STATES.input_required.color}
          note="waiting on a person"
        />
        <Tile
          label="Waiting on others"
          value={c.blocked}
          color="var(--blocked)"
          note="blocked by another node"
        />
        <Tile
          label="Silent too long"
          value={c.stale}
          color="var(--stale)"
          note="working, but no word in 30 min"
        />
        <Tile
          label="Events held"
          value={r.event_count.toLocaleString()}
          note={span ? `over ${duration(span)}` : undefined}
        />
      </div>

      <div className="grid two">
        <div className="panel">
          <h2>States</h2>
          <p className="sub">
            Every node, by what it&rsquo;s doing. Hover a slice.
          </p>
          <div className="row" style={{gap: 28, alignItems: "center"}}>
            <Donut
              slices={STATE_ORDER.map(s => ({
                key: s,
                label: STATES[s].label,
                value: c.by_state[s],
                color: STATES[s].color,
              }))}
            />
            <div style={{flex: 1, display: "grid", gap: 8}}>
              {STATE_ORDER.map(s => (
                <div key={s} className="row" style={{gap: 10}}>
                  <span
                    style={{
                      width: 10,
                      height: 10,
                      borderRadius: 3,
                      background: STATES[s].color,
                    }}
                  />
                  <span style={{flex: 1}}>{STATES[s].label}</span>
                  <b className="mono">{c.by_state[s]}</b>
                  <span
                    className="faint mono"
                    style={{width: 44, textAlign: "right"}}>
                    {c.nodes ? Math.round((100 * c.by_state[s]) / c.nodes) : 0}%
                  </span>
                </div>
              ))}
            </div>
          </div>
        </div>
        <div className="panel">
          <h2>Activity</h2>
          <p className="sub">
            {plural(events.length, "event")}, from{" "}
            {r.first_event_created ? clock(r.first_event_created) : "–"} to{" "}
            {r.last_event_created ? clock(r.last_event_created) : "–"}. A live
            share keeps its recent history: this is how far back{" "}
            <code>as_of</code> can reach.
          </p>
          <ActivityBars times={events.map(e => e.created)} height={170} />
        </div>
      </div>

      <div className="panel">
        <h2>Sessions</h2>
        <p className="sub">
          Each tree&rsquo;s root, most recently active first. Click one to
          explore everything it started.
        </p>
        <div className="list">
          {[...roots]
            .sort((a, b) => b.last_event - a.last_event)
            .map(n => (
              <NodeRow
                key={n.id}
                node={n}
                onClick={() => onRoot(n.id)}
                right={
                  <span
                    className="faint"
                    style={{fontSize: 12.5, whiteSpace: "nowrap"}}>
                    {n.descendant_count ? `${n.descendant_count} below · ` : ""}
                    {ago(n.last_event)}
                  </span>
                }
              />
            ))}
        </div>
      </div>
    </div>
  );
}
