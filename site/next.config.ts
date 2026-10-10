import {readFileSync} from "node:fs";

import type {NextConfig} from "next";

/**
 * This build's id, from public/version.json (scripts/write-version.mjs, run
 * before each build), compiled into its pages (lib/app-version.ts), so an
 * open page can tell when the file names a newer one. None if it's not
 * there: the pages then never check.
 */
function buildId(): string {
  try {
    const file = new URL("./public/version.json", import.meta.url);
    return JSON.parse(readFileSync(file, "utf8")).version ?? "";
  } catch {
    return "";
  }
}

const nextConfig: NextConfig = {
  env: {NEXT_PUBLIC_BUILD_ID: buildId()},
  // Logs are only read and written through the API, never exposed directly.
  poweredByHeader: false,
  // The graph API runs the viewer's reducer (lib/api/reducer.ts), which it
  // reads from public/: a function has only what it's told it needs.
  outputFileTracingIncludes: {
    "/api/v1/**": ["./public/viewer/agent_graph.wasm"],
  },
  async rewrites() {
    // The API reference: a static site in public/docs/reference, built by
    // ../api-docs, each page an index.html in its own directory. A file that
    // exists (a script, a stylesheet) is served before these apply.
    return [
      {source: "/docs/reference", destination: "/docs/reference/index.html"},
      {
        source: "/docs/reference/:path*",
        destination: "/docs/reference/:path*/index.html",
      },
    ];
  },
  async headers() {
    return [
      {
        source: "/(.*)",
        headers: [
          {key: "X-Content-Type-Options", value: "nosniff"},
          {key: "X-Frame-Options", value: "DENY"},
          {key: "Referrer-Policy", value: "no-referrer"},
        ],
      },
      {
        // The live build's id: never cached (app/app-updates.tsx).
        source: "/version.json",
        headers: [{key: "Cache-Control", value: "no-store, max-age=0"}],
      },
      {
        // The service worker: always fetched fresh, so a new one's seen at
        // once (public/sw.js).
        source: "/sw.js",
        headers: [
          {key: "Cache-Control", value: "no-cache, no-store, must-revalidate"},
          {key: "Content-Type", value: "application/javascript; charset=utf-8"},
        ],
      },
      {
        // Shared logs aren't for search engines.
        source: "/l/:id",
        headers: [{key: "X-Robots-Tag", value: "noindex, nofollow"}],
      },
    ];
  },
};

export default nextConfig;
