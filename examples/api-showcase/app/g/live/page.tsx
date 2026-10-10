"use client";

import {useEffect, useRef, useState} from "react";

import {Donut} from "@/components/charts/Donut";
import {ErrorBox, Explainer, Tile} from "@/components/ui";
import {GraphPage} from "@/lib/graph";
import {DEMO} from "@/lib/demo";
import {useApi} from "@/lib/client";
import {NOISY, eventDetail, eventKind, short} from "@/lib/events";
import {STATE_ORDER, STATES, ago, clock} from "@/lib/format";
import type {AgentEvent, Graph, List} from "@/lib/types";

const EVERY = [2, 4, 10];

export default function Page() {
  return <GraphPage>{graph => <Live graph={graph} />}</GraphPage>;
}

function Live({graph}: {graph: string}) {
  const {get} = useApi();
  const [events, setEvents] = useState<Array<AgentEvent>>([]);
  const [g, setG] = useState<Graph | null>(null);
  const [error, setError] = useState<Error | null>(null);
  const [paused, setPaused] = useState(false);
  const [every, setEvery] = useState(4);
  const [polls, setPolls] = useState({
    count: 0,
    unchanged: 0,
    newEvents: 0,
    last: 0,
  });
  const cursor = useRef<string | null>(null);
  const etag = useRef<string | null>(null);

  // The latest events, to start from.
  useEffect(() => {
    let live = true;
    (async () => {
      try {
        const {data} = await get<List<AgentEvent>>(`/graphs/${graph}/events`, {
          limit: 25,
        });
        if (!live) return;
        setEvents(data!.data);
        cursor.current = data!.data[0]?.id ?? null;
        const first = await get<Graph>(`/graphs/${graph}`);
        if (!live) return;
        setG(first.data);
        etag.current = first.etag;
      } catch (e) {
        if (live) setError(e as Error);
      }
    })();
    return () => {
      live = false;
    };
  }, [graph, get]);

  // Then, every few seconds: anything newer than the last event seen, and the graph, if it's changed.
  useEffect(() => {
    if (paused || error) return;
    const t = setInterval(async () => {
      try {
        const graphRes = await get<Graph>(
          `/graphs/${graph}`,
          {},
          {etag: etag.current},
        );
        let fresh: Array<AgentEvent> = [];
        if (graphRes.status !== 304) {
          setG(graphRes.data);
          etag.current = graphRes.etag;
          const {data} = await get<List<AgentEvent>>(
            `/graphs/${graph}/events`,
            {
              order: "asc",
              limit: 100,
              ...(cursor.current ? {starting_after: cursor.current} : {}),
            },
          );
          fresh = data!.data;
          if (fresh.length) {
            cursor.current = fresh[fresh.length - 1].id;
            setEvents(e => [...fresh.reverse(), ...e].slice(0, 200));
          }
        }
        setPolls(p => ({
          count: p.count + 1,
          unchanged: p.unchanged + (graphRes.status === 304 ? 1 : 0),
          newEvents: p.newEvents + fresh.length,
          last: Date.now(),
        }));
      } catch (e) {
        setError(e as Error);
      }
    }, every * 1000);
    return () => clearInterval(t);
  }, [graph, get, paused, every, error]);

  return (
    <>
      <Explainer
        kicker="Live feed"
        title="Follow it as it happens"
        lede="Agents start, finish, wait and ask for help every few seconds. Follow the graph live, cheaply, with nothing but plain GET requests."
        what={
          <>
            <p>
              Every {every} seconds it asks for the graph with the{" "}
              <code>ETag</code> it last saw. While nothing&rsquo;s changed, the
              API answers <b>304 Not Modified</b>: no body, and it doesn&rsquo;t
              count against your rate limit.
            </p>
            <p>
              When something has, it asks only for the events{" "}
              <b>after the last one it saw</b> (<code>starting_after</code>),
              each with what it changed.
            </p>
          </>
        }
        why={
          <p>
            It&rsquo;s how you&rsquo;d build an alert (&ldquo;an agent needs
            you&rdquo;), a team dashboard, or a bot that watches for a failing
            build and steps in. No webhooks to host, no websocket to keep open:
            one cheap request every few seconds.
          </p>
        }
        calls={[
          "GET /graphs/{id} (If-None-Match)",
          "GET /graphs/{id}/events?order=asc&starting_after=evt_…",
        ]}
      />
      {DEMO ? (
        <div className="callout" style={{marginBottom: 18}}>
          <span>✦</span>
          <span>
            <b>The demo&rsquo;s graph doesn&rsquo;t change,</b> so nothing new
            arrives here: every check is a <code>304</code>. Run this app with
            your own key and it follows your agents as they work.
          </span>
        </div>
      ) : null}
      {error ? <ErrorBox error={error} /> : null}
      <div className="grid side">
        <div className="panel">
          <div className="row" style={{marginBottom: 6}}>
            <span
              style={{
                width: 10,
                height: 10,
                borderRadius: "50%",
                background: paused ? "var(--faint)" : "var(--good)",
                boxShadow: paused ? undefined : "0 0 12px var(--good)",
              }}
              className={paused ? undefined : "pulse"}
            />
            <b>{paused ? "Paused" : "Live"}</b>
            <span className="faint" style={{fontSize: 13}}>
              {polls.last
                ? `checked ${ago(polls.last, Date.now())}`
                : "waiting for the first check"}
            </span>
            <div className="spacer" />
            {EVERY.map(s => (
              <button
                key={s}
                className={`chip ${every === s ? "on" : ""}`}
                onClick={() => setEvery(s)}>
                every {s}s
              </button>
            ))}
            <button className="btn" onClick={() => setPaused(p => !p)}>
              {paused ? "▶ Resume" : "❚❚ Pause"}
            </button>
          </div>
          {!events.length ? <div className="empty">No events yet.</div> : null}
          <div>
            {events.map(e => (
              <FeedItem key={e.id} event={e} />
            ))}
          </div>
        </div>
        <div
          className="stack"
          style={{alignSelf: "start", position: "sticky", top: 20}}>
          <div className="panel">
            <h2>Right now</h2>
            <p className="sub">
              {g ? `As of ${g.as_of_event_id?.slice(0, 16)}…` : "…"}
            </p>
            {g ? (
              <div style={{display: "grid", placeItems: "center"}}>
                <Donut
                  size={200}
                  slices={STATE_ORDER.map(s => ({
                    key: s,
                    label: STATES[s].label,
                    value: g.counts.by_state[s],
                    color: STATES[s].color,
                  }))}
                />
              </div>
            ) : null}
          </div>
          <div className="tiles">
            <Tile label="Checks" value={polls.count} />
            <Tile
              label="304 Not Modified"
              value={polls.unchanged}
              color="var(--accent-2)"
              note="free"
            />
            <Tile
              label="New events"
              value={polls.newEvents}
              color="var(--good)"
            />
          </div>
        </div>
      </div>
    </>
  );
}

function FeedItem({event}: {event: AgentEvent}) {
  const k = eventKind(event.type);
  const detail = eventDetail(event);
  const own = event.changes.find(c => c.node_id === event.node_id);
  const fields = Object.entries(own?.fields ?? {}).filter(
    ([f]) => !NOISY.has(f),
  );
  const state = own?.fields.state?.to as keyof typeof STATES | undefined;
  return (
    <div className="feed-item">
      <span
        className="feed-icon"
        style={
          state
            ? {borderColor: STATES[state].color, color: STATES[state].color}
            : undefined
        }>
        {k.icon}
      </span>
      <div style={{minWidth: 0}}>
        <div>
          <b>{event.type}</b>{" "}
          <span className="faint">· {event.source.provider}</span>
        </div>
        {detail ? (
          <div
            className="muted"
            style={{
              fontSize: 13.5,
              overflow: "hidden",
              textOverflow: "ellipsis",
            }}>
            {detail}
          </div>
        ) : null}
        {fields.length ? (
          <div className="diff">
            {fields.slice(0, 5).map(([f, v]) => (
              <span key={f}>
                <b>{f}</b>{" "}
                {own?.created ? null : (
                  <span className="from">{short(v.from, f)}</span>
                )}{" "}
                <span className="to">{short(v.to, f)}</span>
              </span>
            ))}
            {event.changes.length > 1 ? (
              <span>+{event.changes.length - 1} more nodes</span>
            ) : null}
          </div>
        ) : null}
      </div>
      <span
        className="faint mono"
        style={{fontSize: 11.5, whiteSpace: "nowrap"}}>
        {clock(event.created)}
      </span>
    </div>
  );
}
