import {installScript, latestRelease} from "@/lib/release";

export const dynamic = "force-static";

/**
 * GET /install.sh: installs the latest release (release.json) on macOS or
 * Linux. What it does: `installScript` in lib/release.ts.
 */
export function GET(): Response {
  return new Response(installScript(latestRelease()), {
    headers: {
      "Content-Type": "text/x-shellscript; charset=utf-8",
      // Half a minute: after a release, the script that installs it should
      // be what's served, not the last one, within moments.
      "Cache-Control": "public, max-age=30",
    },
  });
}
