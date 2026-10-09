#!/usr/bin/env node
// Builds the API reference, ../api-docs, into public/docs/reference (served
// at /docs/reference: see next.config.ts). It isn't committed: `build` makes
// it fresh each time, from ../site/openapi.json and ../stainless/stainless.yml,
// installing ../api-docs's packages first if they aren't there.
//
// --if-missing (for `dev`): only if there's no build yet, so starting the dev
// server stays quick. Run `npm --prefix ../api-docs run build` to rebuild.

import {execFileSync} from "node:child_process";
import {existsSync} from "node:fs";
import {dirname, resolve} from "node:path";
import {fileURLToPath} from "node:url";

const site = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const docs = resolve(site, "../api-docs");
const built = resolve(site, "public/docs/reference/index.html");

if (process.argv.includes("--if-missing") && existsSync(built)) {
  process.exit(0);
}
if (!existsSync(resolve(docs, "package.json"))) {
  console.error(`Can't find ${docs}`);
  process.exit(1);
}

// On Windows npm is npm.cmd, which Node (since the CVE-2024-27980 fix) refuses
// to spawn without a shell (EINVAL). The args are fixed, so a shell is safe.
const win = process.platform === "win32";
const run = args =>
  execFileSync("npm", args, {cwd: docs, stdio: "inherit", shell: win});

if (!existsSync(resolve(docs, "node_modules"))) {
  run(["ci", "--no-audit", "--no-fund"]);
}
run(["run", "build"]);
