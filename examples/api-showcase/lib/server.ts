import "server-only";

/** Where the API is (`AGENT_GRAPH_API_URL`), without a trailing slash. */
export const API_URL = (
  process.env.AGENT_GRAPH_API_URL || "https://agentgraph.chofter.com/api/v1"
).replace(/\/$/, "");

/** Your key, from .env (`AGENT_GRAPH_KEY`); null until it's set. */
export function apiKey(): string | null {
  const key = process.env.AGENT_GRAPH_KEY?.trim();
  return key ? key : null;
}
