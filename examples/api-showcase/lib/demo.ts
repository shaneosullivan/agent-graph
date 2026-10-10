// The site's public showcase (/showcase on agentgraph.chofter.com) is this
// app, built as static pages (see next.config.ts), reading a demo account's
// graphs through the site's own route, with a key the site holds.

/** Whether this is the public showcase, reading the demo account's graphs. */
export const DEMO = process.env.NEXT_PUBLIC_SHOWCASE_DEMO === "1";

/** Where the browser's API calls go: this app's server, or the site's showcase route. */
export const API_BASE = process.env.NEXT_PUBLIC_API_BASE || "/api/ag";

/** This app's source, to run on your own graphs. */
export const SOURCE = "https://github.com/shaneosullivan/agent-graph/tree/main/examples/api-showcase";
