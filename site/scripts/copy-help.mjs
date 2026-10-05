#!/usr/bin/env node
// Copies the CLI's help text, ../docs/cli-help.json (which build.rs compiles
// into the binary), to lib/cli-help.json for the /docs page, and what can go
// wrong, ../docs/troubleshooting.json (which `agent-graph diagnostics` has
// too), to lib/troubleshooting.json for /troubleshooting. The copies aren't
// committed: `dev`, `build` and `typecheck` make fresh ones each time.

import {copyFileSync, existsSync} from "node:fs";
import {dirname, join, resolve} from "node:path";
import {fileURLToPath} from "node:url";

const site = resolve(dirname(fileURLToPath(import.meta.url)), "..");
for (const name of ["cli-help.json", "troubleshooting.json"]) {
  const source = resolve(site, "../docs", name);
  if (!existsSync(source)) {
    console.error(`Can't find ${source}`);
    process.exit(1);
  }
  copyFileSync(source, join(site, "lib", name));
}
