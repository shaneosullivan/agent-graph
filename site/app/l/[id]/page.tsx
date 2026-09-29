import type {Metadata} from "next";
import {cookies} from "next/headers";
import {notFound} from "next/navigation";

import {currentUser} from "@/lib/auth";
import {ID_PATTERN, viewCookieName} from "@/lib/config";
import {safeEqual, viewToken} from "@/lib/crypto";
import {getMeta} from "@/lib/store";

import {Viewer} from "../../viewer";
import {Unlock} from "./unlock";

export const dynamic = "force-dynamic";

export const metadata: Metadata = {
  title: "Shared log · Agent Graph",
  robots: {index: false, follow: false},
};

/**
 * A shared log, in the viewer. An account's live share is only its owner's
 * (they see it at /watch): to anyone else it isn't there.
 */
export default async function LogPage({
  params,
}: {
  params: Promise<{id: string}>;
}) {
  const {id} = await params;
  if (!ID_PATTERN.test(id)) {
    notFound();
  }
  const meta = await getMeta(id);
  if (!meta) {
    notFound();
  }
  if (meta.owner && (await currentUser())?.uid !== meta.owner) {
    notFound();
  }

  if (meta.pw) {
    const cookie = (await cookies()).get(viewCookieName(id))?.value;
    if (!safeEqual(cookie, viewToken(id, meta.pw))) {
      return <Unlock id={id} />;
    }
  }
  return <Viewer id={id} live={meta.source === "watch"} />;
}
