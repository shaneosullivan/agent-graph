import Script from "next/script";

import {VIEWER_SHELL} from "@/lib/viewer-shell";

/**
 * The viewer for log `id`: the same one `agent-graph view` serves
 * (public/viewer/app.js), fed by site-source.js, whose worker
 * (site-worker.js) fetches the log and computes graphs in the browser with
 * the reducer compiled to WebAssembly.
 */
export function Viewer({
  id,
  live,
  url,
  shares,
}: {
  id: string;
  live: boolean;
  /** A log that's a file of its own (an example), read whole from here. */
  url?: string;
  /**
   * At /watch: list the sessions of every one of the account's live shares
   * (its computers and cloud instances), not just this log's (see
   * site-source.js).
   */
  shares?: boolean;
}) {
  // Only our own values go in here, but escape `<` anyway so nothing in it
  // could ever close the script tag.
  const config = JSON.stringify({
    id,
    live,
    ...(url ? {url} : {}),
    ...(shares ? {shares} : {}),
  }).replace(/</g, "\\u003c");
  return (
    <>
      <link rel="stylesheet" href="/viewer/app.css" precedence="viewer" />
      {/* Our own static markup, generated from src/view/assets/index.html. */}
      <div
        style={{height: "100%"}}
        dangerouslySetInnerHTML={{__html: VIEWER_SHELL}}
      />
      <script
        id="agent-graph-config"
        type="application/json"
        dangerouslySetInnerHTML={{__html: config}}
      />
      <Script src="/viewer/site-source.js" strategy="afterInteractive" />
    </>
  );
}
