# Agent Graph API showcase

A Next.js app that reads your coding agents' graphs through the [Agent Graph API](https://agentgraph.chofter.com/docs/reference), and shows what that makes possible. Every view explains what it's showing, why it's worth having, and exactly which API calls it makes.

| View              | What it shows                                                                             | What it uses                                                 |
| ----------------- | ----------------------------------------------------------------------------------------- | ------------------------------------------------------------ |
| **Overview**      | Where everything stands: states, what needs you, activity over time                       | `GET /graphs/{id}`, `?is_root=true`, events                  |
| **Tree explorer** | Every session and the agents it started, as a tree you can fold, search and click into    | `expand[]=descendants`: a whole tree in one call             |
| **Time machine**  | The graph's history: scrub, play, and see the graph as it was at any event                | `as_of`, and each event's `changes`                          |
| **Bottlenecks**   | What's stuck, who's waiting on whom, loops of waits, and what's holding up the most work  | Search, and `blocked=true` with `expand[]=data.blocked.on`   |
| **Performance**   | Where the time goes: a timeline, how long each kind of agent takes, how much runs at once | One list in tree order                                       |
| **Live feed**     | New events as they happen, cheaply                                                        | `If-None-Match` (a `304` costs nothing) and `starting_after` |
| **Search**        | The query language, with examples                                                         | `GET …/nodes/search`                                         |
| **AI briefing**   | A briefing in plain words, and a compact, model-ready snapshot to hand another AI         | Six calls, in parallel                                       |

Open **Under the hood** at the foot of any page for every call it made, with its status, its request id, and a `curl` command to make it yourself.

## Run it

You need Node 20.9 or later, and an Agent Graph account with a live share: run `agent-graph watch-remote` on a computer where you use a coding agent. A share stays listed after it stops, until it's had no new events for a week.

1. Make an API key on your [account page](https://agentgraph.chofter.com/account#api-keys), under **API keys**. A secret key (`ag_sk_live_…`) reads every graph; a restricted one (`ag_rk_live_…`) only the shares you chose.
2. Copy `.env.example` to `.env`, and put your key in it:

   ```bash
   cp .env.example .env
   ```

   ```
   AGENT_GRAPH_KEY=ag_sk_live_…
   ```

3. Install and start it:

   ```bash
   npm install
   ```

   ```bash
   npm run dev
   ```

4. Open http://localhost:3200.

`.env` is in this app's `.gitignore`: your key is never committed.

## How it calls the API

Your key stays on the server. The browser calls this app's own route, `app/api/ag/[...path]/route.ts`, which adds the key and passes the request on to `AGENT_GRAPH_API_URL` (the production API unless you change it). It only reads, and only `/graphs`.

- `lib/client.tsx`: the browser's client: `get`, `getAll` (follows a list's cursor, keeping every page at the moment of the first), and the call log behind **Under the hood**.
- `lib/types.ts`: the API's objects, as its spec describes them.
- `components/TreeView.tsx`, `components/charts/*`: the visualisations, drawn with [d3](https://d3js.org).

Text that agents write (titles, summaries, what they're doing) is untrusted: it can contain anything. The AI briefing fences it off before handing it to a model, and you should too.

## The demo at /showcase

The site serves this app at [agentgraph.chofter.com/showcase](https://agentgraph.chofter.com/showcase), reading a demo account's graphs. `SHOWCASE_EXPORT=1 npm run build` builds it that way (the site's `scripts/build-showcase.mjs` does, and copies `out/` into the site): static pages under `/showcase`, without the proxy route, calling the site's own proxy instead, which holds the demo account's key. In that build (`lib/demo.ts`), the pages say they're a demo, and read each graph as it stood at its last event, so it looks the same whenever it's opened.

A view's graph is in its address (`/g/explorer?graph=gph_…`), not its path, so every page can be built ahead of time.
