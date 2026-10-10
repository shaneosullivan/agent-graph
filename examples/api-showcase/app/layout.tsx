import "./globals.css";

import type {Metadata} from "next";
import {Inter, JetBrains_Mono} from "next/font/google";

import {Shell} from "@/components/Shell";
import {CallLogProvider} from "@/lib/client";
import {API_URL} from "@/lib/server";

const sans = Inter({subsets: ["latin"], variable: "--font-sans"});
const mono = JetBrains_Mono({subsets: ["latin"], variable: "--font-mono"});

export const metadata: Metadata = {
  title: "Agent Graph API · showcase",
  description:
    "What your coding agents' graphs tell you, and what you can build on the Agent Graph API.",
};

export default function RootLayout({children}: {children: React.ReactNode}) {
  return (
    <html lang="en" className={`${sans.variable} ${mono.variable}`}>
      <body>
        <CallLogProvider>
          <Shell apiUrl={API_URL}>{children}</Shell>
        </CallLogProvider>
      </body>
    </html>
  );
}
