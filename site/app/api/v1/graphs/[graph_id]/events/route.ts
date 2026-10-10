// GET /api/v1/graphs/{graph_id}/events: see lib/api/v1.ts and site/openapi.json.
export {listEvents as GET} from "@/lib/api/routes";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";
