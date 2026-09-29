#!/usr/bin/env node
// Copies the CLI's help text, ../docs/cli-help.json (which build.rs compiles
// into the binary), to lib/cli-help.json for the /docs page. The copy isn't
// committed: `dev`, `build` and `typecheck` make a fresh one each time.

import {copyFileSync, existsSync} from "node:fs";
import {dirname, join, resolve} from "node:path";
import {fileURLToPath} from "node:url";

const site = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const source = resolve(site, "../docs/cli-help.json");

if (!existsSync(source)) {
  console.error(`Can't find the help text at ${source}`);
  process.exit(1);
}
copyFileSync(source, join(site, "lib/cli-help.json"));
