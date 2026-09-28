import "../site.css";

import type { Metadata } from "next";
import { redirect } from "next/navigation";

import { watchLog } from "@/lib/accounts";
import { currentUser } from "@/lib/auth";
import { getMeta } from "@/lib/store";

import { SiteHeader } from "../site-header";
import { Viewer } from "../viewer";

export const dynamic = "force-dynamic";

export const metadata: Metadata = {
  title: "Live · Agent Graph",
  robots: { index: false, follow: false },
};

/**
 * The account's live share (the latest `agent-graph watch-remote` started
 * or carried on), in the viewer. Only its owner can see it: others are
 * asked to log in, and come back here.
 */
export default async function Watch() {
  const user = await currentUser();
  if (!user) redirect("/login?next=/watch");
  const id = await watchLog(user.uid);
  const meta = id ? await getMeta(id) : null;
  if (id && meta && meta.owner === user.uid) return <Viewer id={id} live />;
  return (
    <div className="site">
      <div className="page">
        <SiteHeader links={[{ href: "/docs", label: "Docs" }]} />
        <div className="card narrow">
          <div className="card-body">
            <h1>Nothing shared yet</h1>
            <p>
              Run <code>agent-graph watch-remote</code> on your computer. Once it&rsquo;s logged in as{" "}
              <strong>{user.email ?? "you"}</strong>, what your agents are doing shows here, live. A share
              with no new events for a week is deleted.
            </p>
          </div>
        </div>
      </div>
    </div>
  );
}
