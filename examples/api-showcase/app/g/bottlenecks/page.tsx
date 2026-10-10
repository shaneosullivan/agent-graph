"use client";

import {useMemo, useState} from "react";

import {WaitGraph} from "@/components/charts/WaitGraph";
import {NodeDrawer} from "@/components/NodeDrawer";
import {ErrorBox, Explainer, Loading, NodeRow, Tile} from "@/components/ui";
import {GraphPage} from "@/lib/graph";
import {useApi, useLoad} from "@/lib/client";
import {now as clockNow, STATES, duration, nodeName, plural} from "@/lib/format";
import type {AgentNode, List, SearchResult} from "@/lib/types";

export default function Page() {
  return <GraphPage>{graph => <Bottlenecks graph={graph} />}</GraphPage>;
}

function Bottlenecks({graph}: {graph: string}) {
  const {get} = useApi();
  const [selected, setSelected] = useState<string | null>(null);

  const {data, error, loading} = useLoad(async () => {
    const [needYou, stale, blocked] = await Promise.all([
      get<SearchResult<AgentNode>>(`/graphs/${graph}/nodes/search`, {
        query: 'state:"input_required"',
        limit: 100,
      }),
      get<SearchResult<AgentNode>>(`/graphs/${graph}/nodes/search`, {
        query: 'stale:"true"',
        limit: 100,
      }),
      get<List<AgentNode>>(`/graphs/${graph}/nodes`, {
        blocked: true,
        expand: ["data.blocked.on"],
        limit: 1000,
      }),
    ]);
    return {
      needYou: needYou.data!,
      stale: stale.data!,
      blocked: blocked.data!.data,
    };
  }, [graph]);

  // What's holding the most up: for each node waited on, who waits on it, and the open tasks queued behind them.
  const ranking = useMemo(() => {
    if (!data) return [];
    const behind = new Map<
      string,
      {node: AgentNode; waiters: Array<AgentNode>; tasks: number}
    >();
    for (const w of data.blocked) {
      for (const o of w.blocked?.on ?? []) {
        const r = behind.get(o.id) ?? {node: o, waiters: [], tasks: 0};
        r.waiters.push(w);
        r.tasks += (w.blocked?.open_tasks ?? 0) + w.task_counts.open;
        behind.set(o.id, r);
      }
    }
    return [...behind.values()].sort(
      (a, b) => b.tasks - a.tasks || b.waiters.length - a.waiters.length,
    );
  }, [data]);

  const now = clockNow();
  const loops = data?.blocked.filter(b => b.blocked?.cycle) ?? [];
  const top = ranking[0];
  const maxTasks = Math.max(1, ...ranking.map(r => r.tasks));

  return (
    <>
      <Explainer
        kicker="Bottlenecks"
        title="What's stuck, and what it's holding up"
        lede="Agents wait: on a person, on a helper they started, on each other. The API knows who's waiting on whom, and how much work is queued behind each wait."
        what={
          <>
            <p>
              Two <b>searches</b> find the nodes waiting on a person and the
              ones that have gone quiet. One <b>list</b>, filtered to{" "}
              <code>blocked=true</code> with{" "}
              <code>expand[]=data.blocked.on</code>, returns every waiting node
              <i> with the nodes it waits on</i> in the same reply: that&rsquo;s
              the whole graph of waits, drawn below.
            </p>
            <p>
              Each waiting node also says how many open tasks stand between it
              and carrying on.
            </p>
          </>
        }
        why={
          <p>
            This is where an optimiser earns its keep. One slow agent can hold
            up a whole tree; a loop of waits never finishes at all. Finding them
            by hand means reading every transcript. Here it&rsquo;s three
            requests, and the ranking tells you which one wait, cleared, frees
            the most work.
          </p>
        }
        calls={[
          'GET …/nodes/search?query=state:"input_required"',
          'GET …/nodes/search?query=stale:"true"',
          "GET …/nodes?blocked=true&expand[]=data.blocked.on",
        ]}
      />
      {loading ? <Loading what="Looking for what's stuck" /> : null}
      {error ? <ErrorBox error={error} /> : null}
      {data ? (
        <div className="grid side">
          <div className="stack">
            <div className="tiles">
              <Tile
                label="Need you"
                value={data.needYou.total_count}
                color={STATES.input_required.color}
                note="waiting on a person"
              />
              <Tile
                label="Silent too long"
                value={data.stale.total_count}
                color="var(--stale)"
                note="may have hung"
              />
              <Tile
                label="Waiting on others"
                value={data.blocked.length}
                color="var(--blocked)"
              />
              <Tile
                label="Tasks queued behind waits"
                value={data.blocked.reduce(
                  (s, b) => s + (b.blocked?.open_tasks ?? 0),
                  0,
                )}
                note="open tasks in what's waited on"
              />
            </div>

            {top ? (
              <div className="callout">
                <span>💡</span>
                <span>
                  <b>{nodeName(top.node)}</b> is the biggest bottleneck:{" "}
                  {plural(top.waiters.length, "node")}{" "}
                  {top.waiters.length === 1 ? "is" : "are"} waiting on it, with{" "}
                  {plural(top.tasks, "open task")} between them.{" "}
                  {top.node.stale
                    ? "And it's gone quiet: it may have hung."
                    : ""}
                </span>
              </div>
            ) : null}
            {loops.length ? (
              <div className="callout warn">
                <span>⟳</span>
                <span>
                  <b>A loop of waits:</b> {loops.map(nodeName).join(", ")} wait
                  on each other. Nothing in a loop can ever finish: one of them
                  has to be stopped.
                </span>
              </div>
            ) : null}

            <div className="panel">
              <h2>Who&rsquo;s waiting on whom</h2>
              <p className="sub">
                Arrows point from a waiting node down to what it waits on; what
                everything waits for is at the bottom. Click a node for its
                details.
              </p>
              <WaitGraph waiting={data.blocked} onPick={setSelected} />
            </div>

            <div className="panel">
              <h2>Biggest bottlenecks</h2>
              <p className="sub">
                Nodes others wait on, by the open work queued behind them.
              </p>
              {!ranking.length ? (
                <div className="empty">Nothing is waited on.</div>
              ) : null}
              {ranking.slice(0, 8).map(r => (
                <div
                  key={r.node.id}
                  style={{marginBottom: 12, cursor: "pointer"}}
                  onClick={() => setSelected(r.node.id)}>
                  <div className="row" style={{fontSize: 14}}>
                    <span
                      style={{
                        width: 9,
                        height: 9,
                        borderRadius: "50%",
                        background: STATES[r.node.state].color,
                      }}
                    />
                    <b>{nodeName(r.node)}</b>
                    <span className="faint">{STATES[r.node.state].label}</span>
                    <div className="spacer" />
                    <span className="muted mono">
                      {r.waiters.length} waiting · {plural(r.tasks, "task")}
                    </span>
                  </div>
                  <div className="progress" style={{marginTop: 6, height: 9}}>
                    <div
                      style={{
                        width: `${(100 * r.tasks) / maxTasks}%`,
                        background:
                          "linear-gradient(90deg, var(--blocked), var(--accent))",
                      }}
                    />
                  </div>
                </div>
              ))}
            </div>

            <div className="grid two">
              <div className="panel">
                <h2>Waiting on you</h2>
                <p className="sub">What each is asking: answer these first.</p>
                {!data.needYou.data.length ? (
                  <div className="empty">
                    Nothing&rsquo;s waiting on a person.
                  </div>
                ) : null}
                <div className="list">
                  {data.needYou.data.map(n => (
                    <NodeRow
                      key={n.id}
                      node={n}
                      onClick={() => setSelected(n.id)}
                    />
                  ))}
                </div>
              </div>
              <div className="panel">
                <h2>Possibly hung</h2>
                <p className="sub">
                  Working, waiting on nothing, and silent for 30 minutes or
                  more.
                </p>
                {!data.stale.data.length ? (
                  <div className="empty">Nothing&rsquo;s gone quiet.</div>
                ) : null}
                <div className="list">
                  {data.stale.data.map(n => (
                    <NodeRow
                      key={n.id}
                      node={n}
                      onClick={() => setSelected(n.id)}
                      right={
                        <span
                          className="mono"
                          style={{
                            color: "var(--stale)",
                            fontSize: 12.5,
                            whiteSpace: "nowrap",
                          }}>
                          silent {duration(now - n.last_event)}
                        </span>
                      }
                    />
                  ))}
                </div>
              </div>
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
              Click any node to see what it&rsquo;s doing, what it&rsquo;s
              waiting on, and what started it.
            </div>
          )}
        </div>
      ) : null}
    </>
  );
}
