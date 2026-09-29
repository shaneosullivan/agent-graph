import type {MetadataRoute} from "next";

/**
 * The web app manifest (/manifest.webmanifest), for installing the site as
 * an app: its icons are made by scripts/make-icons.py.
 */
export default function manifest(): MetadataRoute.Manifest {
  return {
    name: "Agent Graph",
    short_name: "Agent Graph",
    description:
      "A live, step-through view of your AI coding agents: which session started which agents, who's waiting on whom, and what needs you.",
    id: "/",
    start_url: "/",
    scope: "/",
    display: "standalone",
    background_color: "#f6f6f3",
    theme_color: "#f6f6f3",
    icons: [
      {src: "/icons/icon-192.png", sizes: "192x192", type: "image/png"},
      {src: "/icons/icon-512.png", sizes: "512x512", type: "image/png"},
      {
        src: "/icons/maskable-512.png",
        sizes: "512x512",
        type: "image/png",
        purpose: "maskable",
      },
    ],
  };
}
