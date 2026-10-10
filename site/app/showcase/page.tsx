"use client";

import Link from "next/link";

import {Donut} from "@/app/showcase/_components/charts/Donut";
import {SECTIONS} from "@/app/showcase/_components/Shell";
import {ErrorBox, Loading} from "@/app/showcase/_components/ui";
import {graphHref} from "@/app/showcase/_lib/graph";
import {useSource} from "@/app/showcase/_lib/source";
import {AgentGraphError, useApi, useLoad} from "@/app/showcase/_lib/client";
import {STATE_ORDER, STATES, ago, plural} from "@/app/showcase/_lib/format";
import type {Graph, List} from "@/app/showcase/_lib/types";

export default function Home() {
  const {get} = useApi();
  const {source, signedIn} = useSource();
  const demo = source === "demo";
  const {data, error, loading} = useLoad(async () => {
    const list = (await get<List<Graph>>("/graphs")).data!;
    if (!demo) {
      return list;
    }
    // The demo's graphs, each as it stood at its last event (_lib/client.tsx).
    const pinned = await Promise.all(
      list.data.map(async g => (await get<Graph>(`/graphs/${g.id}`)).data ?? g),
    );
    return {...list, data: pinned};
  }, []);
  const first = data?.data[0]?.id;

  return (
    <>
      <section style={{margin: "8px 0 34px", maxWidth: 900}}>
        <div
          className="kicker"
          style={{
            color: "var(--accent-2)",
            fontSize: 12,
            letterSpacing: "0.1em",
            fontWeight: 600,
          }}>
          THE AGENT GRAPH API
        </div>
        <h1
          style={{
            fontSize: 46,
            lineHeight: 1.08,
            letterSpacing: "-0.035em",
            margin: "10px 0 14px",
          }}>
          Your coding agents,{" "}
          <span
            style={{
              background:
                "linear-gradient(90deg, var(--accent), var(--accent-2))",
              WebkitBackgroundClip: "text",
              color: "transparent",
            }}>
            as data you can build on.
          </span>
        </h1>
        <p className="muted" style={{fontSize: 18, margin: 0}}>
          Every session and every agent it started. What each is doing,
          who&rsquo;s waiting on whom, and what&rsquo;s stuck, at any moment in
          their history. This page reads it all through the API, and shows what
          that makes possible.
        </p>
        <div className="grid three" style={{marginTop: 24}}>
          {[
            [
              "See",
              "The whole tree of work, live: every agent, what it's for, and how it's going.",
            ],
            [
              "Find",
              "What's stuck, what's waiting on a person, and the one agent holding up ten others.",
            ],
            [
              "Feed",
              "A compact, trustworthy snapshot for another system to act on: an optimiser, an alert, an AI.",
            ],
          ].map(([h, p]) => (
            <div key={h} className="panel" style={{padding: "14px 16px"}}>
              <b style={{color: "var(--accent-2)"}}>{h}</b>
              <div className="muted" style={{fontSize: 14, marginTop: 4}}>
                {p}
              </div>
            </div>
          ))}
        </div>
      </section>

      {demo ? (
        <div className="callout" style={{marginBottom: 24}}>
          <span>✦</span>
          <span>
            <b>This is a demo.</b> It reads a demo account&rsquo;s graphs
            through the API, live, as they stood at their last events.{" "}
            {signedIn ? (
              <>
                To see your own here, switch to <b>Your data</b>, at the top
                right.{" "}
              </>
            ) : (
              <>
                <a href="/login?next=/showcase">Log in</a> to see your own
                here.{" "}
              </>
            )}
            To read them from your own code, make a key on your{" "}
            <a href="/account#api-keys">account page</a>, and see the{" "}
            <a href="/docs/reference">API reference</a>.
          </span>
        </div>
      ) : null}

      <h2 style={{fontSize: 20, margin: "0 0 4px"}}>
        {demo ? "The demo\u2019s graphs" : "Your graphs"}
      </h2>
      <p className="muted" style={{margin: "0 0 16px", fontSize: 14}}>
        Each is a live share (<code>agent-graph watch-remote</code>), running or
        not, from <code>GET /graphs</code>. Pick one to explore it.
      </p>

      {loading ? <Loading what="Listing your graphs" /> : null}
      {error ? <Setup error={error} /> : null}
      {data && !data.data.length ? (
        <div className="empty">
          No live shares yet. Run <code>agent-graph watch-remote</code> on a
          computer where you use a coding agent, and its graph appears here. It
          stays listed after it stops, until it&rsquo;s had no new events for a
          week.
        </div>
      ) : null}
      <div className="grid two">
        {data?.data.map(g => (
          <Link
            key={g.id}
            href={graphHref(g.id)}
            className="panel"
            style={{color: "inherit", textDecoration: "none"}}>
            <div className="row" style={{alignItems: "center", gap: 18}}>
              <Donut
                size={120}
                slices={STATE_ORDER.map(s => ({
                  key: s,
                  label: STATES[s].label,
                  value: g.counts.by_state[s],
                  color: STATES[s].color,
                }))}
              />
              <div style={{flex: 1, minWidth: 0}}>
                <div
                  className="mono"
                  style={{fontSize: 13, color: "var(--accent)"}}>
                  {g.id}
                </div>
                <div
                  style={{
                    fontSize: 22,
                    fontWeight: 650,
                    letterSpacing: "-0.02em",
                    margin: "2px 0",
                  }}>
                  {plural(g.counts.sessions, "session")},{" "}
                  {plural(g.counts.agents, "agent")}
                </div>
                <div className="muted" style={{fontSize: 13.5}}>
                  Last active {ago(g.last_event)} ·{" "}
                  {plural(g.retention.event_count, "event")} held
                </div>
                <div className="row" style={{marginTop: 10, gap: 6}}>
                  {g.counts.by_state.input_required ? (
                    <span
                      className="badge"
                      style={
                        {
                          "--c": STATES.input_required.color,
                        } as React.CSSProperties
                      }>
                      {g.counts.by_state.input_required} need you
                    </span>
                  ) : null}
                  {g.counts.blocked ? (
                    <span
                      className="badge"
                      style={{"--c": "var(--blocked)"} as React.CSSProperties}>
                      {g.counts.blocked} waiting on others
                    </span>
                  ) : null}
                  {g.counts.stale ? (
                    <span
                      className="badge"
                      style={{"--c": "var(--stale)"} as React.CSSProperties}>
                      {g.counts.stale} silent too long
                    </span>
                  ) : null}
                </div>
              </div>
              <span className="btn primary">Open →</span>
            </div>
          </Link>
        ))}
      </div>

      <h2 style={{fontSize: 20, margin: "40px 0 4px"}}>What&rsquo;s in here</h2>
      <p className="muted" style={{margin: "0 0 16px", fontSize: 14}}>
        Eight views, each built on a different part of the API, each explaining
        what it calls and why.
      </p>
      <div className="grid three">
        {SECTIONS.map(s => {
          const body = (
            <>
              <div style={{fontSize: 22}}>{s.icon}</div>
              <b>{s.label}</b>
              <div className="muted" style={{fontSize: 13.5}}>
                {s.blurb}
              </div>
            </>
          );
          return first ? (
            <Link
              key={s.slug}
              href={graphHref(first, s.slug)}
              className="panel"
              style={{color: "inherit", textDecoration: "none"}}>
              {body}
            </Link>
          ) : (
            <div key={s.slug} className="panel" style={{opacity: 0.6}}>
              {body}
            </div>
          );
        })}
      </div>
    </>
  );
}

function Setup({error}: {error: Error}) {
  const code = error instanceof AgentGraphError ? error.code : null;
  if (code === "not_logged_in") {
    return (
      <div className="empty">
        You&rsquo;re no longer logged in here.{" "}
        <a href="/login?next=/showcase">Log in again</a> to read your own
        graphs.
      </div>
    );
  }
  if (code !== "showcase_not_configured") {
    return <ErrorBox error={error} />;
  }
  return (
    <div className="empty">
      The showcase isn&rsquo;t set up yet: it has no key to read the demo
      account with.
    </div>
  );
}
