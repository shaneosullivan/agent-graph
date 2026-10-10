// GET /api/v1/graphs/{graph_id}/nodes/search: see lib/api/v1.ts and site/openapi.json.
export {searchNodes as GET} from "@/lib/api/routes";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";
