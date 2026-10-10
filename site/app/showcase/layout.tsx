import "./showcase.css";

import type {Metadata} from "next";
import {Inter, JetBrains_Mono} from "next/font/google";

import {Shell} from "./_components/Shell";
import {CallLogProvider} from "./_lib/client";
import {SourceProvider} from "./_lib/source";

const sans = Inter({subsets: ["latin"], variable: "--font-sans"});
const mono = JetBrains_Mono({subsets: ["latin"], variable: "--font-mono"});

export const metadata: Metadata = {
  title: "Agent Graph API · showcase",
  description:
    "What your coding agents' graphs tell you, and what you can build on the Agent Graph API: a demo account's graphs, read through it.",
};

/** The API, as the curl commands under "Under the hood" call it. */
const API_URL = `${(process.env.NEXT_PUBLIC_SITE_URL || "https://agentgraph.chofter.com").replace(/\/+$/, "")}/api/v1`;

/**
 * The API showcase: a demo account's graphs, read through the graph API
 * (by way of the site's proxy, app/showcase/api/ag, which holds the demo's
 * key: lib/showcase.ts), with what each view shows, why, and the calls it
 * makes. Its look is its own, scoped to this wrapper (showcase.css).
 */
export default function ShowcaseLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <div className={`showcase ${sans.variable} ${mono.variable}`}>
      <SourceProvider>
        <CallLogProvider>
          <Shell apiUrl={API_URL}>{children}</Shell>
        </CallLogProvider>
      </SourceProvider>
    </div>
  );
}
