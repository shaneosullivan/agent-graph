import {readFile} from "node:fs/promises";
import {join} from "node:path";

import {ImageResponse} from "next/og";

/**
 * The image shown where a link to the site is shared (Open Graph, and X's
 * card: app/twitter-image.tsx): the logo (public/icons/logo-512.png, made
 * by scripts/make-icons.py), the name and what it does. Made once, at
 * build time.
 */
export const alt = "Agent Graph: Lead your army of agents with confidence.";
export const size = {width: 1200, height: 630};
export const contentType = "image/png";

export default async function Image() {
  const logo = await readFile(
    join(process.cwd(), "public/icons/logo-512.png"),
    "base64",
  );
  return new ImageResponse(
    <div
      style={{
        width: "100%",
        height: "100%",
        display: "flex",
        alignItems: "center",
        gap: 64,
        padding: "0 96px",
        background: "#f6f6f3",
        color: "#1c1c1e",
      }}>
      <img
        src={`data:image/png;base64,${logo}`}
        width={340}
        height={340}
        alt=""
      />
      <div
        style={{
          display: "flex",
          flexDirection: "column",
          gap: 24,
          flex: 1,
          minWidth: 0,
        }}>
        <div style={{fontSize: 96, letterSpacing: -2}}>Agent Graph</div>
        <div style={{fontSize: 40, lineHeight: 1.3, color: "#6b6b72"}}>
          See what your AI coding agents are doing, live, and share it.
        </div>
      </div>
    </div>,
    size,
  );
}
