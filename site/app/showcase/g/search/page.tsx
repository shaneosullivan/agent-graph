"use client";

import {useEffect, useState} from "react";

import {NodeDrawer} from "@/app/showcase/_components/NodeDrawer";
import {
  ErrorBox,
  Explainer,
  Loading,
  NodeRow,
} from "@/app/showcase/_components/ui";
import {GraphPage} from "@/app/showcase/_lib/graph";
import {AgentGraphError, useApi} from "@/app/showcase/_lib/client";
import {nodeName} from "@/app/showcase/_lib/format";
import type {AgentNode, SearchResult} from "@/app/showcase/_lib/types";

const EXAMPLES: Array<{q: string; why: string}> = [
  {q: 'state:"input_required"', why: "Everything waiting on a person"},
  {q: 'stale:"true"', why: "Working, but silent for 30 minutes"},
  {
    q: 'state:"working" AND blocked:"true"',
    why: "Busy, but held up by another node",
  },
  {q: 'state:"failed" OR state:"canceled"', why: "What didn't finish"},
  {q: 'kind:"agent" AND depth>=2', why: "Agents started by agents"},
  {q: "descendant_count>3", why: "Nodes that fanned out"},
  {q: "blocked_open_tasks>=3", why: "Work queued behind a wait"},
  {q: 'summary~"test"', why: "Anything whose status mentions tests"},
];

const FIELDS =
  "state kind provider agent_type title summary headline purpose attention stale blocked background parent_id root_id session_id depth created ended last_event child_count descendant_count open_tasks blocked_open_tasks";

export default function Page() {
  return <GraphPage>{graph => <Search graph={graph} />}</GraphPage>;
}

function Search({graph}: {graph: string}) {
  const {get} = useApi();
  const [query, setQuery] = useState(EXAMPLES[0].q);
  const [withRoot, setWithRoot] = useState(true);
  const [result, setResult] = useState<SearchResult<AgentNode> | null>(null);
  const [items, setItems] = useState<Array<AgentNode>>([]);
  const [error, setError] = useState<Error | null>(null);
  const [busy, setBusy] = useState(false);
  const [selected, setSelected] = useState<string | null>(null);

  async function run(q = query, page?: string) {
    setBusy(true);
    setError(null);
    try {
      const {data} = await get<SearchResult<AgentNode>>(
        `/graphs/${graph}/nodes/search`,
        {
          query: q,
          limit: 25,
          ...(withRoot ? {expand: ["data.root"]} : {}),
          ...(page ? {page} : {}),
        },
      );
      setResult(data);
      setItems(i => (page ? [...i, ...data!.data] : data!.data));
    } catch (e) {
      setError(e as Error);
      if (!page) {
        setResult(null);
        setItems([]);
      }
    }
    setBusy(false);
  }

  // Show what the first example finds, straight away.
  useEffect(() => {
    // It searches, and shows what it found as it comes back.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    run();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [graph]);

  const queryError =
    error instanceof AgentGraphError && error.code === "search_query_invalid"
      ? error
      : null;

  return (
    <>
      <Explainer
        kicker="Search"
        title="Ask the graph a question"
        lede="A small query language over every node: match states and types, compare counts and times, look for words in what agents say they're doing."
        what={
          <>
            <p>
              Clauses like <code>field:&quot;value&quot;</code>,{" "}
              <code>field~&quot;text&quot;</code> (contains),{" "}
              <code>field&gt;3</code> and <code>-field:&quot;value&quot;</code>{" "}
              (not), joined with <code>AND</code> or <code>OR</code>. Results
              come most recently active first, with a <code>total_count</code>,
              a page at a time (<code>next_page</code>), every page as of the
              same moment.
            </p>
            <p>
              Ticking <i>with each tree&rsquo;s root</i> adds{" "}
              <code>expand[]=data.root</code>: each result arrives with its root
              node beside its <code>root_id</code>.
            </p>
          </>
        }
        why={
          <p>
            Questions a list filter can&rsquo;t answer: OR, negation,
            thresholds, text. It&rsquo;s what an agent of your own would ask
            before acting (&ldquo;which Explore agents have been at it
            longest?&rdquo;), and a bad query says exactly what&rsquo;s wrong
            with it, so a model can fix its own.
          </p>
        }
        calls={["GET /graphs/{id}/nodes/search?query=…"]}
      />
      <div className="grid side">
        <div className="stack">
          <div className="panel">
            <form
              className="row"
              onSubmit={e => {
                e.preventDefault();
                run();
              }}>
              <input
                type="text"
                value={query}
                onChange={e => setQuery(e.target.value)}
                className="mono"
                style={{flex: 1, fontSize: 14}}
                spellCheck={false}
              />
              <button className="btn primary" type="submit" disabled={busy}>
                Search
              </button>
            </form>
            <label
              className="row muted"
              style={{fontSize: 13, marginTop: 10, gap: 6}}>
              <input
                type="checkbox"
                checked={withRoot}
                onChange={e => setWithRoot(e.target.checked)}
              />{" "}
              with each tree&rsquo;s root (<code>expand[]=data.root</code>)
            </label>
            <div className="row" style={{marginTop: 14, gap: 8}}>
              {EXAMPLES.map(x => (
                <button
                  key={x.q}
                  className={`chip ${query === x.q ? "on" : ""}`}
                  title={x.q}
                  onClick={() => {
                    setQuery(x.q);
                    run(x.q);
                  }}>
                  {x.why}
                </button>
              ))}
            </div>
            {queryError ? (
              <div className="callout warn" style={{marginTop: 14}}>
                <span>✎</span>
                <span>
                  <b>The API says:</b> {queryError.message}
                </span>
              </div>
            ) : null}
          </div>
          {error && !queryError ? <ErrorBox error={error} /> : null}
          <div className="panel">
            {result ? (
              <h2>
                {result.total_count.toLocaleString()} match
                {result.total_count === 1 ? "" : "es"}
                <span className="faint" style={{fontWeight: 400, fontSize: 13}}>
                  {" "}
                  · as of {result.as_of_event_id?.slice(0, 18)}…
                </span>
              </h2>
            ) : (
              <h2>Pick an example, or write a query</h2>
            )}
            {busy && !items.length ? <Loading what="Searching" /> : null}
            <div className="list">
              {items.map(n => (
                <NodeRow
                  key={n.id}
                  node={n}
                  onClick={() => setSelected(n.id)}
                  right={
                    n.root && n.root.id !== n.id ? (
                      <span
                        className="faint"
                        style={{
                          fontSize: 12,
                          maxWidth: 200,
                          textAlign: "right",
                        }}>
                        in {nodeName(n.root)}
                      </span>
                    ) : null
                  }
                />
              ))}
            </div>
            {result?.next_page ? (
              <button
                className="btn"
                style={{marginTop: 12}}
                disabled={busy}
                onClick={() => run(query, result.next_page!)}>
                Next page
              </button>
            ) : null}
          </div>
          <details className="panel">
            <summary style={{cursor: "pointer"}}>
              <b>Every field you can search</b>
            </summary>
            <p
              className="mono muted"
              style={{lineHeight: 1.9, marginBottom: 0}}>
              {FIELDS.split(" ").map(f => (
                <code key={f} style={{marginRight: 6, display: "inline-block"}}>
                  {f}
                </code>
              ))}
            </p>
          </details>
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
            Click a result to see it in full.
          </div>
        )}
      </div>
    </>
  );
}
