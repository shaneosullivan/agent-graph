// Builds the viewer's D3 (../src/view/assets/d3.min.js, which
// sync-viewer.mjs copies to public/viewer/): only the parts of D3 the graph
// view uses (app.js, "graph view"), bundled and minified, with the
// licences of the packages it's made from. Its output is committed, so
// building the site or the binary never needs this; run it after changing
// what the graph view uses, or the D3 packages' versions:
//
//   npm run build-d3

import {readFileSync, writeFileSync} from "node:fs";
import {dirname, join} from "node:path";
import {fileURLToPath} from "node:url";

import {build} from "esbuild";

const site = join(dirname(fileURLToPath(import.meta.url)), "..");
const out = join(site, "../src/view/assets/d3.min.js");

// What app.js uses as `d3.…` (d3-transition adds `.transition()` to
// selections, so it's imported for that alone).
const entry = `
export {forceLink, forceManyBody, forceSimulation, forceX, forceY} from "d3-force";
export {select} from "d3-selection";
import "d3-transition";
export {zoom, zoomIdentity, zoomTransform} from "d3-zoom";
export {drag} from "d3-drag";
export {easeBackIn, easeBackOut, easeCubicInOut, easeCubicOut} from "d3-ease";
export {timer} from "d3-timer";
`;

const result = await build({
  stdin: {contents: entry, resolveDir: site, loader: "js"},
  bundle: true,
  minify: true,
  format: "iife",
  globalName: "d3",
  target: "es2019",
  metafile: true,
  write: false,
  legalComments: "none",
});

// Each package in the bundle (by its folder), with its licence.
const packages = new Map();
for (const input of Object.keys(result.metafile.inputs)) {
  const m = input.match(/^(.*node_modules\/((?:@[^/]+\/)?[^/]+))\//);
  if (m) packages.set(m[2], join(site, m[1]));
}
// Each licence once, with the packages under it (most of D3's share one).
const byText = new Map();
for (const [name, dir] of [...packages].sort()) {
  const {version, license} = JSON.parse(
    readFileSync(join(dir, "package.json"), "utf8"),
  );
  const text = readFileSync(join(dir, "LICENSE"), "utf8").trim();
  if (!byText.has(text)) byText.set(text, {license, names: []});
  byText.get(text).names.push(`${name} ${version}`);
}
const notices = [...byText].map(
  ([text, {license, names}]) => `${names.join(", ")} (${license}):\n\n${text}`,
);
const banner =
  "/*! The parts of D3 (https://d3js.org) the viewer's graph view uses, built\n" +
  " * by site/scripts/build-d3.mjs from these packages, under their licences:\n *\n" +
  notices
    .join("\n\n---\n\n")
    .split("\n")
    .map(line => ` * ${line}`.trimEnd())
    .join("\n") +
  "\n */\n";

writeFileSync(out, banner + result.outputFiles[0].text);
console.log(
  `${out}: ${(banner.length + result.outputFiles[0].text.length) / 1000} kB, from ${[...packages.keys()].join(", ")}`,
);
