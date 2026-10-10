// GET /api/v1/graphs: see lib/api/v1.ts and site/openapi.json.
export {listGraphs as GET} from "@/lib/api/routes";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";
