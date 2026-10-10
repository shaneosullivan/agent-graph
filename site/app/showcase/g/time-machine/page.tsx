"use client";

import {useEffect, useMemo, useState} from "react";

import {
  StackedArea,
  type Point,
} from "@/app/showcase/_components/charts/StackedArea";
import {NodeDrawer} from "@/app/showcase/_components/NodeDrawer";
import {TreeView} from "@/app/showcase/_components/TreeView";
import {
  ErrorBox,
  Explainer,
  Legend,
  Loading,
} from "@/app/showcase/_components/ui";
import {GraphPage} from "@/app/showcase/_lib/graph";
import {useApi, useLoad} from "@/app/showcase/_lib/client";
import {NOISY, eventDetail, eventKind, short} from "@/app/showcase/_lib/events";
import {STATE_ORDER, clock, nodeName} from "@/app/showcase/_lib/format";
import type {AgentEvent, AgentNode, NodeState} from "@/app/showcase/_lib/types";

const empty = (): Record<NodeState, number> =>
  Object.fromEntries(STATE_ORDER.map(s => [s, 0])) as Record<NodeState, number>;

export default function Page() {
  return <GraphPage>{graph => <TimeMachine graph={graph} />}</GraphPage>;
}

function TimeMachine({graph}: {graph: string}) {
  const {getAll} = useApi();

  // The history: every event it holds, oldest first; and the graph as it stood at the first.
  const history = useLoad(async () => {
    const events = (
      await getAll<AgentEvent>(`/graphs/${graph}/events`, {order: "asc"}, 5)
    ).data;
    const start = events.length
      ? (
          await getAll<AgentNode>(`/graphs/${graph}/nodes`, {
            as_of: events[0].id,
          })
        ).data
      : [];
    return {events, start};
  }, [graph]);

  // Replaying each event's changes gives every state, at every event, without another call.
  const points = useMemo<Array<Point>>(() => {
    if (!history.data) {
      return [];
    }
    const {events, start} = history.data;
    const state = new Map(start.map(n => [n.id, n.state]));
    const count = (): Record<NodeState, number> => {
      const c = empty();
      for (const s of state.values()) {
        c[s] += 1;
      }
      return c;
    };
    return events.map((e, i) => {
      if (i > 0) {
        for (const ch of e.changes) {
          const to = ch.fields.state?.to as NodeState | undefined;
          if (to) {
            state.set(ch.node_id, to);
          }
        }
      }
      return {t: e.created, counts: count()};
    });
  }, [history.data]);

  const n = history.data?.events.length ?? 0;
  const [at, setAt] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [settled, setSettled] = useState(0);
  const [selected, setSelected] = useState<string | null>(null);

  // Start at the newest event, once they're here (set as it renders:
  // React's way to reset state when what it's from changes).
  const [atFor, setAtFor] = useState(n);
  if (atFor !== n) {
    setAtFor(n);
    setAt(Math.max(0, n - 1));
  }
  // Fetch the moment once the scrubber settles.
  useEffect(() => {
    const t = setTimeout(() => setSettled(at), playing ? 0 : 280);
    return () => clearTimeout(t);
  }, [at, playing]);
  useEffect(() => {
    if (!playing) {
      return;
    }
    const t = setInterval(
      () =>
        setAt(a => {
          if (a >= n - 1) {
            setPlaying(false);
            return a;
          }
          return a + 1;
        }),
      700,
    );
    return () => clearInterval(t);
  }, [playing, n]);

  const event = history.data?.events[settled];
  const moment = useLoad(async () => {
    if (!event) {
      return null;
    }
    return (
      await getAll<AgentNode>(`/graphs/${graph}/nodes`, {
        as_of: event.id,
        order: "tree",
      })
    ).data;
  }, [graph, event?.id]);

  const changed = useMemo(
    () => new Set(event?.changes.map(c => c.node_id) ?? []),
    [event],
  );
  const names = useMemo(
    () => new Map((moment.data ?? []).map(m => [m.id, nodeName(m)])),
    [moment.data],
  );
  const current = history.data?.events[at];

  return (
    <>
      <Explainer
        kicker="Time machine"
        title="Replay the graph, event by event"
        lede="Every read in the API can be as of any moment the graph still holds. Scrub through its history and watch agents start, wait, finish and fail."
        what={
          <>
            <p>
              The chart is how many nodes were in each state after every event,
              worked out here by replaying each event&rsquo;s{" "}
              <code>changes</code> (every field it changed, from what to what),
              with no call per step.
            </p>
            <p>
              When you stop on a moment, the tree is fetched{" "}
              <b>as of that event</b>: <code>as_of=evt_…</code> gives the graph
              exactly as it was then. Nodes the event changed flash.
            </p>
          </>
        }
        why={
          <p>
            Post-mortems, without guessing: what was running when that build
            broke, what an agent was waiting on before it was cancelled, how
            long a session sat waiting for a person. And because a fixed moment
            never changes, its replies are cached for good (
            <code>Cache-Control: immutable</code>).
          </p>
        }
        calls={[
          "GET /graphs/{id}/events?order=asc",
          "GET /graphs/{id}/nodes?as_of=evt_…&order=tree",
        ]}
      />
      {history.loading ? <Loading what="Loading the history" /> : null}
      {history.error ? <ErrorBox error={history.error} /> : null}
      {history.data && !n ? (
        <div className="empty">This graph holds no events yet.</div>
      ) : null}
      {history.data && n ? (
        <div className="stack">
          <div className="panel">
            <div className="row">
              <button
                className="btn primary"
                onClick={() =>
                  at >= n - 1
                    ? (setAt(0), setPlaying(true))
                    : setPlaying(p => !p)
                }>
                {playing ? "❚❚ Pause" : "▶ Play"}
              </button>
              <button
                className="btn"
                onClick={() => setAt(a => Math.max(0, a - 1))}>
                ← Back
              </button>
              <button
                className="btn"
                onClick={() => setAt(a => Math.min(n - 1, a + 1))}>
                Next →
              </button>
              <div className="spacer" />
              <span className="mono muted" style={{fontSize: 13}}>
                event {at + 1} of {n} · {current ? clock(current.created) : ""}
              </span>
            </div>
            <div style={{marginTop: 14}}>
              <StackedArea
                points={points}
                at={at}
                onPick={i => (setPlaying(false), setAt(i))}
              />
            </div>
            <input
              type="range"
              min={0}
              max={n - 1}
              value={at}
              onChange={e => (setPlaying(false), setAt(Number(e.target.value)))}
            />
            <Legend />
          </div>

          <div className="grid side">
            <div className="panel">
              <h2>The graph, as of this event</h2>
              <p className="sub">
                <code>as_of={event?.id}</code>
              </p>
              {moment.error ? <ErrorBox error={moment.error} /> : null}
              {moment.data ? (
                <TreeView
                  nodes={moment.data}
                  flash={changed}
                  selected={selected}
                  onSelect={m => setSelected(m.id)}
                  pending={moment.loading || settled !== at}
                  height={520}
                />
              ) : (
                <Loading what="Fetching that moment" />
              )}
            </div>
            <div className="stack">
              {event ? (
                <EventCard event={event} names={names} onSelect={setSelected} />
              ) : null}
              {selected && event ? (
                <NodeDrawer
                  graph={graph}
                  nodeId={selected}
                  asOf={event.id}
                  onSelect={setSelected}
                  onClose={() => setSelected(null)}
                />
              ) : null}
            </div>
          </div>
        </div>
      ) : null}
    </>
  );
}

function EventCard({
  event,
  names,
  onSelect,
}: {
  event: AgentEvent;
  names: Map<string, string>;
  onSelect: (id: string) => void;
}) {
  const [all, setAll] = useState(false);
  const k = eventKind(event.type);
  const detail = eventDetail(event);
  return (
    <div className="panel">
      <div className="row" style={{gap: 12}}>
        <span className="feed-icon">{k.icon}</span>
        <div style={{flex: 1, minWidth: 0}}>
          <div>
            <a
              href="#"
              onClick={e => (e.preventDefault(), onSelect(event.node_id))}>
              {names.get(event.node_id) ?? "A node"}
            </a>{" "}
            {k.verb}
          </div>
          <div className="faint mono" style={{fontSize: 12}}>
            {event.type} · {clock(event.created)}
          </div>
        </div>
      </div>
      {detail ? (
        <p style={{margin: "10px 0 0", color: "#c8cde0", fontSize: 14}}>
          {detail}
        </p>
      ) : null}
      <h3
        style={{
          margin: "14px 0 6px",
          fontSize: 12,
          letterSpacing: "0.08em",
          textTransform: "uppercase",
          color: "var(--faint)",
        }}>
        What it changed
      </h3>
      {!event.changes.length ? (
        <div className="faint">Nothing: it repeated what was already so.</div>
      ) : null}
      {event.changes.map(c => {
        const fields = Object.entries(c.fields).filter(
          ([f]) => all || !NOISY.has(f),
        );
        return (
          <div key={c.node_id} style={{marginBottom: 10}}>
            <a
              href="#"
              onClick={e => (e.preventDefault(), onSelect(c.node_id))}
              style={{fontSize: 13.5}}>
              {names.get(c.node_id) ?? c.node_id.slice(0, 18)}
            </a>
            {c.created ? (
              <span className="faint" style={{fontSize: 12}}>
                {" "}
                · new
              </span>
            ) : null}
            <div className="diff">
              {fields.slice(0, c.created && !all ? 6 : 20).map(([f, v]) => (
                <span key={f}>
                  <b>{f}</b>{" "}
                  {c.created ? null : (
                    <span className="from">{short(v.from, f)}</span>
                  )}{" "}
                  <span className="to">{short(v.to, f)}</span>
                </span>
              ))}
            </div>
          </div>
        );
      })}
      {event.changes.length ? (
        <button className="chip" onClick={() => setAll(a => !a)}>
          {all ? "Fewer fields" : "Every field"}
        </button>
      ) : null}
    </div>
  );
}
