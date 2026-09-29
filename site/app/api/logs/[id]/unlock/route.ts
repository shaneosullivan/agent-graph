import {unlock} from "@/lib/unlock";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

/** Unlocks a password-protected log for this browser: see lib/unlock.ts. */
export async function POST(
  req: Request,
  {params}: {params: Promise<{id: string}>},
): Promise<Response> {
  return unlock(req, (await params).id);
}
