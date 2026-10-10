"use client";

import {useRouter, useSearchParams} from "next/navigation";
import {useState} from "react";

import {NodeDrawer} from "@/app/showcase/_components/NodeDrawer";
import {usePhone} from "@/app/showcase/_lib/useWidth";
import {TreeView} from "@/app/showcase/_components/TreeView";
import {
  ErrorBox,
  Explainer,
  Legend,
  Loading,
} from "@/app/showcase/_components/ui";
import {GraphPage, graphHref} from "@/app/showcase/_lib/graph";
import {useApi, useLoad} from "@/app/showcase/_lib/client";
import {STATES, nodeName} from "@/app/showcase/_lib/format";
import type {AgentNode, List} from "@/app/showcase/_lib/types";

export default function Page() {
  return <GraphPage>{graph => <Explorer graph={graph} />}</GraphPage>;
}

const EVERYTHING = "everything";

function Explorer({graph}: {graph: string}) {
  const search = useSearchParams();
  const router = useRouter();
  const {get, getAll} = useApi();
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState<string | null>(null);

  const roots = useLoad(
    async () =>
      (
        await get<List<AgentNode>>(`/graphs/${graph}/nodes`, {
          is_root: true,
          limit: 200,
        })
      ).data!.data.sort(
        (a, b) =>
          b.descendant_count - a.descendant_count ||
          b.last_event - a.last_event,
      ),
    [graph],
  );
  const root = search.get("root") ?? roots.data?.[0]?.id ?? null;

  // One tree: the root, with everything under it, in one call. Or the whole forest, as a list in tree order.
  const tree = useLoad(async () => {
    if (!root) {
      return null;
    }
    if (root === EVERYTHING) {
      return (
        await getAll<AgentNode>(`/graphs/${graph}/nodes`, {order: "tree"})
      ).data;
    }
    const {data} = await get<AgentNode>(`/graphs/${graph}/nodes/${root}`, {
      expand: ["descendants"],
    });
    const {descendants, ...top} = data!;
    return [top as AgentNode, ...(descendants?.data ?? [])];
  }, [graph, root]);

  // A new root is the node picked (set as it renders: React's way to reset
  // state when a prop changes); not on a phone, where its details would
  // cover the tree.
  const phone = usePhone();
  const [selectedFor, setSelectedFor] = useState(root);
  if (selectedFor !== root) {
    setSelectedFor(root);
    setSelected(root && root !== EVERYTHING && !phone ? root : null);
  }

  // Picking a tree changes the address, but not where you are on the page.
  const pick = (id: string) =>
    router.replace(graphHref(graph, "explorer", {root: id}), {scroll: false});

  return (
    <>
      <Explainer
        kicker="Tree explorer"
        title="Every agent, and what it started"
        lede="Sessions start agents, which start agents of their own. Here's the whole tree: fold and unfold it, search it, and click any node for everything about it."
        what={
          <>
            <p>
              A session&rsquo;s whole tree comes back in <b>one call</b>: the
              root, with <code>expand[]=descendants</code>, is every node under
              it as a flat list in tree order, each with its{" "}
              <code>parent_id</code>, which is all it takes to draw it.
            </p>
            <p>
              Click a dot to fold or unfold what&rsquo;s under it; click a name
              for the node in full, retrieved with its tasks, waits, messages
              and children expanded in the same call.
            </p>
          </>
        }
        why={
          <p>
            It&rsquo;s the map of what your agents are actually doing: which
            session fanned out to how many helpers, which branch is still
            working, which one failed. Dot size is how much is under a node;
            colour is its state; a dashed orange ring means it&rsquo;s gone
            quiet; a purple dot, that it&rsquo;s waiting on another.
          </p>
        }
        calls={[
          "GET /graphs/{id}/nodes?is_root=true",
          "GET /graphs/{id}/nodes/{node_id}?expand[]=descendants",
          "GET /graphs/{id}/nodes?order=tree",
          "GET /graphs/{id}/nodes/{node_id}?expand[]=…",
        ]}
      />
      {roots.error ? <ErrorBox error={roots.error} /> : null}
      {phone ? (
        <label className="picker" style={{marginBottom: 14}}>
          <span className="faint">Tree</span>
          <select
            value={root ?? EVERYTHING}
            onChange={e => pick(e.target.value)}>
            <option value={EVERYTHING}>✦ Everything</option>
            {roots.data?.map(r => (
              <option key={r.id} value={r.id}>
                {nodeName(r).slice(0, 40)}
                {r.descendant_count ? ` (+${r.descendant_count})` : ""} ·{" "}
                {STATES[r.state].label}
              </option>
            ))}
          </select>
        </label>
      ) : (
        <div className="row" style={{marginBottom: 14, gap: 8}}>
          <span className="faint" style={{fontSize: 13}}>
            Tree:
          </span>
          <button
            className={`chip ${root === EVERYTHING ? "on" : ""}`}
            onClick={() => pick(EVERYTHING)}>
            ✦ Everything
          </button>
          {roots.data?.slice(0, 14).map(r => (
            <button
              key={r.id}
              className={`chip ${root === r.id ? "on" : ""}`}
              onClick={() => pick(r.id)}>
              <span
                style={{
                  width: 7,
                  height: 7,
                  borderRadius: "50%",
                  background: STATES[r.state].color,
                }}
              />
              {nodeName(r).slice(0, 28)}
              {r.descendant_count ? (
                <span className="faint">+{r.descendant_count}</span>
              ) : null}
            </button>
          ))}
        </div>
      )}
      <div className="grid side">
        <div className="panel">
          <div className="row" style={{marginBottom: 12}}>
            <input
              type="search"
              placeholder="Highlight nodes: a name, a state, what they're doing…"
              value={query}
              onChange={e => setQuery(e.target.value)}
              style={{flex: 1}}
            />
          </div>
          {tree.loading && !tree.data ? (
            <Loading what="Fetching the tree" />
          ) : null}
          {tree.error ? <ErrorBox error={tree.error} /> : null}
          {tree.data ? (
            <TreeView
              nodes={tree.data}
              selected={selected}
              onSelect={n => setSelected(n.id)}
              query={query}
              foldBelow={tree.data.length > 60 ? 2 : undefined}
              pending={tree.loading}
            />
          ) : null}
          <div style={{marginTop: 12}}>
            <Legend flags />
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
          <div className="panel muted">
            Click a node&rsquo;s name to see everything about it.
          </div>
        )}
      </div>
    </>
  );
}
