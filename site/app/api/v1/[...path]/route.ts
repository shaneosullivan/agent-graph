// Anything else under /api/v1: a 404 in the API's shape: see lib/api/v1.ts and site/openapi.json.
export {unknown as GET} from "@/lib/api/routes";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";
