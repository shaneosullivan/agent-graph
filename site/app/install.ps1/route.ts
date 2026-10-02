import {latestRelease, powerShellScript} from "@/lib/release";

export const dynamic = "force-static";

/**
 * GET /install.ps1: installs the latest release (release.json) on Windows,
 * in PowerShell (`irm …/install.ps1 | iex`). What it does:
 * `powerShellScript` in lib/release.ts.
 */
export function GET(): Response {
  return new Response(powerShellScript(latestRelease()), {
    headers: {
      "Content-Type": "text/plain; charset=utf-8",
      // Half a minute, as /install.sh: after a release, the script that
      // installs it should be what's served, within moments.
      "Cache-Control": "public, max-age=30",
    },
  });
}
