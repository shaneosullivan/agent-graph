import type {Metadata, Viewport} from "next";

const ICON =
  "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 32 32'%3E%3Cpath d='M9 9v14M9 16h14' stroke='%233b5bdb' stroke-width='3' fill='none'/%3E%3Ccircle cx='9' cy='8' r='5' fill='%233b5bdb'/%3E%3Ccircle cx='24' cy='16' r='5' fill='%237c4dff'/%3E%3Ccircle cx='9' cy='24' r='5' fill='%231f9d55'/%3E%3C/svg%3E";

export const metadata: Metadata = {
  metadataBase: new URL(
    process.env.NEXT_PUBLIC_SITE_URL || "https://agentgraph.chofter.com",
  ),
  title: "Agent Graph",
  description:
    "Share a live, step-through view of your AI coding agents: which session started which agents, who's waiting on whom, what's stuck, and what needs you.",
  icons: {icon: ICON},
};

export const viewport: Viewport = {
  width: "device-width",
  initialScale: 1,
};

export default function RootLayout({children}: {children: React.ReactNode}) {
  return (
    <html lang="en">
      <body>{children}</body>
    </html>
  );
}
