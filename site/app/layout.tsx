import type {Metadata, Viewport} from "next";

const DESCRIPTION =
  "Share a live, step-through view of your AI coding agents: which session started which agents, who's waiting on whom, what's stuck, and what needs you.";

export const metadata: Metadata = {
  metadataBase: new URL(
    process.env.NEXT_PUBLIC_SITE_URL || "https://agentgraph.chofter.com",
  ),
  title: "Agent Graph",
  description: DESCRIPTION,
  // The icons, the manifest and the sharing image are files beside this
  // one (app/favicon.ico, icon.svg, apple-icon.png, manifest.ts,
  // opengraph-image.tsx, twitter-image.tsx), which Next links itself.
  openGraph: {
    type: "website",
    siteName: "Agent Graph",
    title: "Agent Graph",
    description: DESCRIPTION,
  },
  twitter: {card: "summary_large_image"},
};

export const viewport: Viewport = {
  width: "device-width",
  initialScale: 1,
  // The page's background (app/site.css), for the browser's own bars.
  themeColor: [
    {media: "(prefers-color-scheme: light)", color: "#f6f6f3"},
    {media: "(prefers-color-scheme: dark)", color: "#131315"},
  ],
};

export default function RootLayout({children}: {children: React.ReactNode}) {
  return (
    <html lang="en">
      <body>{children}</body>
    </html>
  );
}
