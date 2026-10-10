// GET /api/v1/graphs/{graph_id}/nodes/{node_id}: see lib/api/v1.ts and site/openapi.json.
export {retrieveNode as GET} from "@/lib/api/routes";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";
