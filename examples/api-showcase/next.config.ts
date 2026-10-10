import type {NextConfig} from "next";

// SHOWCASE_EXPORT=1 builds the public demo the site serves at /showcase
// (site/scripts/build-showcase.mjs): static pages, with no server of their
// own, so no `route.ts` (only .tsx files count as pages). They call the
// site's own proxy, which holds the demo account's key.
const exporting = process.env.SHOWCASE_EXPORT === "1";

const nextConfig: NextConfig = {
  poweredByHeader: false,
  // In development, React's strict mode runs each effect twice, so every
  // call would show twice under "Under the hood": not what a page asks for.
  reactStrictMode: false,
  ...(exporting
    ? {
        output: "export",
        basePath: "/showcase",
        pageExtensions: ["tsx"],
        images: {unoptimized: true},
        env: {
          NEXT_PUBLIC_SHOWCASE_DEMO: "1",
          NEXT_PUBLIC_API_BASE: "/showcase/api/ag",
        },
      }
    : {}),
};

export default nextConfig;
