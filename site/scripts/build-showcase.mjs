#!/usr/bin/env node
// Builds the API showcase, ../examples/api-showcase, as static pages into
// public/showcase (served at /showcase: see next.config.ts). It isn't
// committed: `build` makes it fresh each time, installing the showcase's
// packages first if they aren't there.
//
// The pages read a demo account's graphs through this site's own proxy,
// app/showcase/api/ag/[...path]/route.ts, which holds that account's key
// (SHOWCASE_API_KEY). The key is never part of the pages.
//
// --if-missing (for `dev`): only if there's no build yet, so starting the dev
// server stays quick. Run `npm run build-showcase` to rebuild.

import {execFileSync} from "node:child_process";
import {cpSync, existsSync, rmSync} from "node:fs";
import {dirname, resolve} from "node:path";
import {fileURLToPath} from "node:url";

const site = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const app = resolve(site, "../examples/api-showcase");
const target = resolve(site, "public/showcase");

if (
  process.argv.includes("--if-missing") &&
  existsSync(resolve(target, "index.html"))
) {
  process.exit(0);
}
if (!existsSync(resolve(app, "package.json"))) {
  console.error(`Can't find ${app}`);
  process.exit(1);
}

// npm as the one running this script, when it's run from npm: on Windows,
// `npm` is a .cmd file, which can't be started without a shell.
const npmCli = process.env.npm_execpath;
const run = args =>
  npmCli && /\.[cm]?js$/.test(npmCli)
    ? execFileSync(process.execPath, [npmCli, ...args], {
        cwd: app,
        stdio: "inherit",
        env,
      })
    : execFileSync("npm", args, {cwd: app, stdio: "inherit", env});

const env = {
  ...process.env,
  SHOWCASE_EXPORT: "1",
  // Shown under "Under the hood", for the curl commands it gives.
  AGENT_GRAPH_API_URL:
    process.env.SHOWCASE_API_URL || "https://agentgraph.chofter.com/api/v1",
  AGENT_GRAPH_KEY: "",
};

if (!existsSync(resolve(app, "node_modules"))) {
  // Its devDependencies too (TypeScript, which `next build` needs), even
  // where NODE_ENV=production would leave them out, as on Vercel.
  run(["ci", "--include=dev", "--no-audit", "--no-fund"]);
}
run(["run", "build"]);

rmSync(target, {recursive: true, force: true});
cpSync(resolve(app, "out"), target, {recursive: true});
console.log(`Built the showcase into ${target}`);
