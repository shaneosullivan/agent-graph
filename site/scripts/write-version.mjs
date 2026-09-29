// Writes public/version.json: this build's id, which the open pages compare
// with their own to tell a newer build is live, and reload
// (app/app-updates.tsx). next.config.ts reads the same file, to compile the
// id into the pages, so the two always match. Run before each build and
// dev server (package.json's prebuild and predev).
//
// The id is Vercel's deployment, or else the commit, or else the time.

import {writeFileSync} from "node:fs";
import {dirname, join} from "node:path";
import {fileURLToPath} from "node:url";

const version =
  process.env.VERCEL_DEPLOYMENT_ID ||
  process.env.VERCEL_GIT_COMMIT_SHA ||
  `local-${Date.now()}`;

const file = join(
  dirname(fileURLToPath(import.meta.url)),
  "../public/version.json",
);
writeFileSync(file, `${JSON.stringify({version})}\n`);
console.log(`Build ${version} (public/version.json)`);
