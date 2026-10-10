// GET /api/v1/graphs/{graph_id}/events/{event_id}: see lib/api/v1.ts and site/openapi.json.
export {retrieveEvent as GET} from "@/lib/api/routes";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";
