// Gathers what the docs are made from, so each has one source:
//
// - The guide (src/content/docs/index.md): the spec's `info.description`,
//   from site/openapi.json.
// - The logo and favicon: the site's own icon, site/app/icon.svg (it picks
//   its light or dark version itself).
//
// The API reference itself is read from the spec and stainless/stainless.yml
// by astro.config.mjs, at build time.
import {copyFileSync, mkdirSync, readFileSync, writeFileSync} from "node:fs";

const root = new URL("../", import.meta.url);
const repo = new URL("../", root);

const spec = JSON.parse(
  readFileSync(new URL("site/openapi.json", repo), "utf8"),
);

// The spec's links are written for a reader of the spec alone (Redoc-style
// fragments); in the docs they're pages. Absolute: the site serves this page
// at /docs/reference, without a trailing slash, so a relative link would
// resolve from /docs.
const api = "/docs/reference/api/resources/graphs";
const links = {
  "#tag/Graphs": `${api}`,
  "#tag/Nodes": `${api}/subresources/nodes`,
  "#tag/Events": `${api}/subresources/events`,
  "#operation/retrieveNode": `${api}/subresources/nodes/methods/retrieve`,
  "#operation/listNodes": `${api}/subresources/nodes/methods/list`,
  "#operation/searchNodes": `${api}/subresources/nodes/methods/search`,
  "#section/Errors": "#errors",
};
const body = spec.info.description.replace(
  /\]\((#[A-Za-z/]+)\)/g,
  (whole, href) => (links[href] ? `](${links[href]})` : whole),
);

const page = `---
title: Overview
description: ${JSON.stringify(spec.info.summary)}
---

<!-- Made by scripts/prepare.mjs from site/openapi.json: edit it there. -->

${body}
`;

mkdirSync(new URL("src/content/docs/", root), {recursive: true});
writeFileSync(new URL("src/content/docs/index.md", root), page);

mkdirSync(new URL("src/assets/", root), {recursive: true});
mkdirSync(new URL("public/", root), {recursive: true});
const icon = new URL("site/app/icon.svg", repo);
copyFileSync(icon, new URL("src/assets/logo.svg", root));
copyFileSync(icon, new URL("public/favicon.svg", root));

console.log("Prepared the guide, logo and favicon.");
