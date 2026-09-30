#!/usr/bin/env node
// Copies what the site shares with the Rust crate, so neither can drift:
//
//   ../src/view/assets/app.js, app.css, d3.min.js  ->  public/viewer/
//   ../src/view/assets/index.html       ->  lib/viewer-shell.ts (its markup)
//
// With --wasm it also builds the reducer to WebAssembly (needs Rust and the
// wasm32-unknown-unknown target) into public/viewer/agent_graph.wasm, and
// records what it was built from (agent_graph.wasm.sources): the crate's
// files the build compiled, the crates it used (versions and features), and
// the build profile. With --check-wasm (which needs Cargo) it only checks
// that record against the repository, so a change to what the WebAssembly
// is built from can't reach the site without a new build (and a change to
// anything else doesn't need one).
//
// The outputs are committed, so building the site doesn't need Rust. With
// --if-present it quietly does nothing when ../src isn't there (e.g. a host
// that only has this folder).

import {spawnSync} from "node:child_process";
import {createHash} from "node:crypto";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import {homedir} from "node:os";
import {dirname, join, relative, resolve, sep} from "node:path";
import {fileURLToPath} from "node:url";

const site = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const crate = resolve(site, "..");
const assets = join(crate, "src/view/assets");
const args = new Set(process.argv.slice(2));

if (!existsSync(assets)) {
  if (args.has("--if-present")) process.exit(0);
  console.error(`Can't find the viewer at ${assets}`);
  process.exit(1);
}

const out = join(site, "public/viewer");
const stamp = join(out, "agent_graph.wasm.sources");

/** A hash of `files` (paths in the crate), ignoring line endings (so a Windows checkout agrees). */
function hashOf(files) {
  const hash = createHash("sha256");
  for (const f of files) {
    hash.update(`${f}\n`);
    hash.update(
      existsSync(join(crate, f))
        ? readFileSync(join(crate, f), "utf8").replace(/\r\n/g, "\n")
        : "(gone)",
    );
  }
  return hash.digest("hex");
}

const wasmArgs = [
  "-p",
  "agent-graph-wasm",
  "--target",
  "wasm32-unknown-unknown",
];

/** The crates the build uses, as "name version features" (the crates here without their paths). */
function crates() {
  // Without colour: CI turns it on, and it wraps the "(*)" marks.
  const args = [
    "tree",
    ...wasmArgs,
    "-e",
    "normal",
    "--prefix",
    "none",
    "--format",
    "{p} {f}",
    "--color",
    "never",
  ];
  const tree = spawnSync("cargo", args, {cwd: crate, encoding: "utf8"});
  if (tree.status !== 0) {
    console.error(
      tree.error
        ? `Can't run cargo (${tree.error.message}); is Rust installed?`
        : tree.stderr,
    );
    process.exit(tree.status ?? 1);
  }
  const lines = tree.stdout
    .split("\n")

    .map(l =>
      l
        .replace(/\x1b\[[0-9;]*m/g, "")
        .replace(/ \([^)]*\)/g, "")
        // This repository's own crates without their versions: what's in
        // them is in `files`, and a release's version bump doesn't change
        // what the WebAssembly's built from (it compiles nothing that
        // reads the version).
        .replace(/^(agent-graph(?:-wasm)?) v\S+/, "$1")
        .trim(),
    );
  return [...new Set(lines.filter(Boolean))].sort();
}

/** The build's profile: `[profile.wasm]`, and the `[profile.release]` it inherits. */
function profiles() {
  const manifest = readFileSync(join(crate, "Cargo.toml"), "utf8").replace(
    /\r\n/g,
    "\n",
  );
  return ["release", "wasm"].map(name => {
    const at = manifest.indexOf(`[profile.${name}]`);
    const end = manifest.indexOf("\n[", at + 1);
    const section = at < 0 ? "" : manifest.slice(at, end < 0 ? undefined : end);
    return section
      .split("\n")
      .filter(l => l.trim() && !l.trim().startsWith("#"))
      .join("\n");
  });
}

if (args.has("--check-wasm")) {
  const record = existsSync(stamp)
    ? JSON.parse(readFileSync(stamp, "utf8"))
    : null;
  const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);
  const problem = !record
    ? "there's no record of what it was built from"
    : hashOf(record.files) !== record.hash
      ? "the crate's files it was built from have changed"
      : !same(record.crates, crates())
        ? "the crates it uses (or their features) have changed"
        : !same(record.profiles, profiles())
          ? "its build profile has changed"
          : null;
  if (problem) {
    console.error(
      `public/viewer/agent_graph.wasm is out of date: ${problem}. In site/, run \`npm run build-wasm\`, and commit it.`,
    );
    // What differs, to see why.
    const now = {crates: crates(), profiles: profiles()};
    for (const key of ["crates", "profiles"]) {
      const was = new Set(record ? record[key] : []);
      const is = new Set(now[key]);
      for (const line of was)
        if (!is.has(line)) console.error(`  was: ${key}: ${line}`);
      for (const line of is)
        if (!was.has(line)) console.error(`  now: ${key}: ${line}`);
    }
    process.exit(1);
  }
  console.log("The site's WebAssembly is built from the crate as it is now.");
  process.exit(0);
}

mkdirSync(out, {recursive: true});
for (const name of ["app.js", "app.css", "d3.min.js"])
  copyFileSync(join(assets, name), join(out, name));

const html = readFileSync(join(assets, "index.html"), "utf8");
const body = html
  .slice(html.indexOf("<body>") + "<body>".length, html.lastIndexOf("</body>"))
  .trim();
if (!body.startsWith('<div class="app">')) {
  console.error('index.html\'s <body> should hold a single <div class="app">');
  process.exit(1);
}
writeFileSync(
  join(site, "lib/viewer-shell.ts"),
  "// Generated by scripts/sync-viewer.mjs from src/view/assets/index.html. Don't edit.\n" +
    `export const VIEWER_SHELL = ${JSON.stringify(body)};\n`,
);

if (args.has("--wasm")) {
  // Paths into the build machine (its home folder, this checkout) are left
  // out of the binary, so it's the same wherever it's built. (Passed
  // separated by \x1f, so a path with a space in it is still one flag.)
  const given = process.env.CARGO_ENCODED_RUSTFLAGS
    ? process.env.CARGO_ENCODED_RUSTFLAGS.split("\x1f")
    : (process.env.RUSTFLAGS || "").split(/\s+/).filter(Boolean);
  const remap = [
    `--remap-path-prefix=${homedir()}=~`,
    `--remap-path-prefix=${crate}=.`,
  ];
  const build = spawnSync(
    "cargo",
    ["build", ...wasmArgs, "--profile", "wasm"],
    {
      cwd: crate,
      stdio: "inherit",
      env: {
        ...process.env,
        CARGO_ENCODED_RUSTFLAGS: [...given, ...remap].join("\x1f"),
      },
    },
  );
  if (build.status !== 0) process.exit(build.status ?? 1);
  const target = join(crate, "target/wasm32-unknown-unknown/wasm");
  copyFileSync(
    join(target, "agent_graph_wasm.wasm"),
    join(out, "agent_graph.wasm"),
  );

  // What it was built from: the crate's files the build compiled (its
  // dep-info), and the crates it used, at their versions.
  const depInfo = readFileSync(join(target, "agent_graph_wasm.d"), "utf8");
  const inputs = depInfo
    .slice(depInfo.indexOf(": ") + 2)
    .trim()
    .split(/(?<!\\) /);
  const files = inputs
    .map(f => relative(crate, f.replace(/\\ /g, " ")).split(sep).join("/"))
    .filter(f => !f.startsWith(".."))
    .sort();
  const record = {
    files,
    crates: crates(),
    profiles: profiles(),
    hash: hashOf(files),
  };
  writeFileSync(stamp, `${JSON.stringify(record, null, 2)}\n`);
}

console.log(
  `Synced the viewer into ${out}${args.has("--wasm") ? " (with WebAssembly)" : ""}.`,
);
