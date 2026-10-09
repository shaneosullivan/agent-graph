#!/usr/bin/env node
// Builds the API reference, ../api-docs, into public/docs/reference (served
// at /docs/reference: see next.config.ts). It isn't committed: `build` makes
// it fresh each time, from ../site/openapi.json and ../stainless/stainless.yml,
// installing ../api-docs's packages first if they aren't there.
//
// --if-missing (for `dev`): only if there's no build yet, so starting the dev
// server stays quick. Run `npm --prefix ../api-docs run build` to rebuild.
//
// Not on Windows: @stainless/sdk-json, which reads the spec, starts its
// worker from a Windows path where it needs a URL, and fails. The site is
// deployed from Linux, where it works; on Windows, /docs/reference is left
// out.

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
if (process.platform === "win32") {
  console.log(
    "Not building the API reference (/docs/reference): it can't be built on Windows.",
  );
  process.exit(0);
}
if (!existsSync(resolve(docs, "package.json"))) {
  console.error(`Can't find ${docs}`);
  process.exit(1);
}

const run = args => execFileSync("npm", args, {cwd: docs, stdio: "inherit"});

if (!existsSync(resolve(docs, "node_modules"))) {
  run(["ci", "--no-audit", "--no-fund"]);
}
run(["run", "build"]);
