#!/usr/bin/env node
// =============================================================================
// npm-packages.mjs
//
// Makes the npm packages for a release, ready for `npm publish`:
//
//   agent-graph                        npm/agent-graph: the command (a Node
//                                      script that runs the program) and its
//                                      README, which is npm's page for it
//   @chofter/agent-graph-<os>-<cpu>    one per build: the program itself,
//                                      for that os and cpu
//
// agent-graph has every platform package as an optional dependency, of
// this version exactly, and npm installs only the one for its machine.
// Nothing else in this repository goes in them.
//
// Usage:
//   node scripts/npm-packages.mjs <version> <out> <target>=<program>...
// e.g.
//   node scripts/npm-packages.mjs 0.1.16 target/npm \
//     aarch64-apple-darwin=target/notarized/aarch64-apple-darwin/agent-graph
//
// Each package is a folder in <out> (replaced). It prints them, one per
// line, in the order to publish them: the platform packages first, so
// agent-graph's dependencies are there before it is.
// =============================================================================

import fs from "node:fs";
import path from "node:path";
import {fileURLToPath} from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

// Rust's targets, as Node names their os and cpu (process.platform,
// process.arch). The Linux builds are static (musl), so they run on any
// Linux, glibc's too.
const PLATFORMS = {
  "aarch64-apple-darwin": {os: "darwin", cpu: "arm64", name: "macOS, Apple silicon"},
  "x86_64-apple-darwin": {os: "darwin", cpu: "x64", name: "macOS, Intel"},
  "aarch64-unknown-linux-musl": {os: "linux", cpu: "arm64", name: "Linux, ARM64"},
  "x86_64-unknown-linux-musl": {os: "linux", cpu: "x64", name: "Linux, x86_64"},
  "aarch64-pc-windows-msvc": {os: "win32", cpu: "arm64", name: "Windows, ARM64"},
  "x86_64-pc-windows-msvc": {os: "win32", cpu: "x64", name: "Windows, x86_64"},
};

function fail(message) {
  console.error(`ERROR: ${message}`);
  process.exit(1);
}

const [version, out, ...builds] = process.argv.slice(2);
if (!version || !out || builds.length === 0) {
  fail("usage: node scripts/npm-packages.mjs <version> <out> <target>=<program>...");
}

// Its version (0.0.0) is a placeholder, and it's private, so npm won't
// publish it as it is: only the copy made here, with the version and its
// platform packages.
const {private: _, ...main} = JSON.parse(
  fs.readFileSync(path.join(root, "npm/agent-graph/package.json"), "utf8"),
);
const license = path.join(root, "LICENSE");

fs.rmSync(out, {recursive: true, force: true});
const dirs = [];
const dependencies = {};

for (const build of builds) {
  const [target, program] = build.split("=");
  const platform = PLATFORMS[target];
  if (!platform) fail(`no npm platform for ${target}`);
  if (!program || !fs.statSync(program, {throwIfNoEntry: false})?.isFile()) {
    fail(`no program for ${target} at ${program}`);
  }
  const name = `@chofter/agent-graph-${platform.os}-${platform.cpu}`;
  const exe = platform.os === "win32" ? "agent-graph.exe" : "agent-graph";
  const dir = path.join(out, `agent-graph-${platform.os}-${platform.cpu}`);
  fs.mkdirSync(path.join(dir, "bin"), {recursive: true});
  fs.copyFileSync(program, path.join(dir, "bin", exe));
  // npm keeps a file's executable bit in the package.
  fs.chmodSync(path.join(dir, "bin", exe), 0o755);
  fs.copyFileSync(license, path.join(dir, "LICENSE"));
  const pkg = {
    name,
    version,
    description: `agent-graph's program for ${platform.name}. Install agent-graph, which installs this.`,
    homepage: main.homepage,
    repository: main.repository,
    license: main.license,
    os: [platform.os],
    cpu: [platform.cpu],
    files: ["bin"],
    // Yarn's Plug'n'Play: a program has to be a real file to run.
    preferUnplugged: true,
  };
  fs.writeFileSync(path.join(dir, "package.json"), JSON.stringify(pkg, null, 2) + "\n");
  fs.writeFileSync(
    path.join(dir, "README.md"),
    `# ${name}\n\nThe [agent-graph](https://www.npmjs.com/package/agent-graph) program for ${platform.name}. ` +
      `Don't install this yourself: install agent-graph, which installs the one for your machine.\n\n` +
      "```bash\nnpm install -g agent-graph\n```\n",
  );
  dependencies[name] = version;
  dirs.push(dir);
}

const dir = path.join(out, "agent-graph");
fs.cpSync(path.join(root, "npm/agent-graph"), dir, {recursive: true});
fs.chmodSync(path.join(dir, "bin/agent-graph.js"), 0o755);
fs.copyFileSync(license, path.join(dir, "LICENSE"));
fs.writeFileSync(
  path.join(dir, "package.json"),
  JSON.stringify({...main, version, optionalDependencies: dependencies}, null, 2) + "\n",
);
dirs.push(dir);

for (const d of dirs) console.log(d);
