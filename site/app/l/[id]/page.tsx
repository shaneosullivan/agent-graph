import type { Metadata } from "next";
import { cookies } from "next/headers";
import { notFound } from "next/navigation";
import Script from "next/script";

import { ID_PATTERN, viewCookieName } from "@/lib/config";
import { safeEqual, viewToken } from "@/lib/crypto";
import { getMeta } from "@/lib/store";
import { VIEWER_SHELL } from "@/lib/viewer-shell";

import { Unlock } from "./unlock";

export const dynamic = "force-dynamic";

export const metadata: Metadata = {
  title: "Shared log · Agent Graph",
  robots: { index: false, follow: false },
};

/**
 * A shared log. The page is the same viewer `agent-graph view` serves
 * (public/viewer/app.js), fed by site-source.js, which fetches the log and
 * computes graphs in the browser with the reducer compiled to WebAssembly.
 */
export default async function LogPage({ params }: { params: Promise<{ id: string }> }) {
  const { id } = await params;
  if (!ID_PATTERN.test(id)) notFound();
  const meta = await getMeta(id);
  if (!meta) notFound();

  if (meta.pw) {
    const cookie = (await cookies()).get(viewCookieName(id))?.value;
    if (!safeEqual(cookie, viewToken(id, meta.pw))) return <Unlock id={id} />;
  }

  // Only our own values go in here, but escape `<` anyway so nothing in it
  // could ever close the script tag.
  const config = JSON.stringify({ id, live: meta.source === "watch" }).replace(/</g, "\\u003c");

  return (
    <>
      <link rel="stylesheet" href="/viewer/app.css" precedence="viewer" />
      {/* Our own static markup, generated from src/view/assets/index.html. */}
      <div style={{ height: "100%" }} dangerouslySetInnerHTML={{ __html: VIEWER_SHELL }} />
      <script id="agent-graph-config" type="application/json" dangerouslySetInnerHTML={{ __html: config }} />
      <Script src="/viewer/site-source.js" strategy="afterInteractive" />
    </>
  );
}
