// The API reference, at https://agentgraph.chofter.com/docs/reference: a
// static site, built into the site's public/ (site/next.config.ts serves
// it), from site/openapi.json and stainless/stainless.yml.
import {defineConfig} from "astro/config";
import {generateAPIReferenceItems, stainlessDocs} from "@stainless-api/docs";
import {fileSystemSDKJSONLoader} from "@stainless-api/docs/plugin";

const repo = new URL("../", import.meta.url);

/**
 * The generated reference sidebar, with every group open (subresources start
 * closed), and the reference's own index page named for what it is, not a
 * second "Overview" beside the guide's.
 */
function openReferenceSidebar(items) {
  for (const item of items) {
    if (item.kind === "api_overview_page") {
      item.label = "API reference";
    }
    if (item.kind === "group") {
      item.collapsed = false;
      openReferenceSidebar(item.entries);
    }
  }
  return items;
}

export default defineConfig({
  site: "https://agentgraph.chofter.com",
  base: "/docs/reference",
  outDir: new URL("site/public/docs/reference/", repo).pathname,
  trailingSlash: "ignore",
  // @stainless-api/docs ships .tsx sources, compiled here; without this
  // they're built for the classic JSX runtime, and fail with "React is not
  // defined".
  vite: {esbuild: {jsx: "automatic", jsxImportSource: "react"}},
  integrations: [
    stainlessDocs({
      title: "Agent Graph API",
      description:
        "Read the graphs your coding agents make, at any point in their history.",
      logo: {src: "./src/assets/logo.svg", alt: "Agent Graph"},
      favicon: "/favicon.svg",
      customCss: ["./theme.css"],
      disableCredits: true,
      header: {
        links: [{label: "Agent Graph", link: "https://agentgraph.chofter.com/"}],
      },
      apiReference: {
        stainlessProject: "agent-graph",
        // Made here, from the spec and config, not fetched from Stainless.
        loadSDKJSONFiles: fileSystemSDKJSONLoader({
          specPath: new URL("site/openapi.json", repo).pathname,
          configFilePath: new URL("stainless/stainless.yml", repo).pathname,
          languages: ["http", "typescript"],
        }),
        defaultLanguage: "http",
        propertySettings: {collapseDescription: false, expandDepth: 1},
      },
      // One sidebar, everywhere: the guide, then every resource and method,
      // all open (it's a short API). (With tabs, the guide's page showed only
      // itself.)
      sidebar: ["", ...generateAPIReferenceItems(openReferenceSidebar)],
    }),
  ],
});
