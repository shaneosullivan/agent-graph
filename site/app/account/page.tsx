import "../site.css";

import type {Metadata} from "next";
import {redirect} from "next/navigation";

import {computersOf, watchLog} from "@/lib/accounts";
import {currentUser} from "@/lib/auth";

import {SiteHeader} from "../site-header";
import {Computers, LogOut} from "./actions";

export const dynamic = "force-dynamic";

export const metadata: Metadata = {
  title: "Account · Agent Graph",
  robots: {index: false, follow: false},
};

/** Who's logged in, their live share, and the computers sharing to it. */
export default async function Account() {
  const user = await currentUser();
  if (!user) {
    redirect("/login?next=/account");
  }
  const [computers, share] = await Promise.all([
    computersOf(user.uid),
    watchLog(user.uid),
  ]);
  return (
    <div className="site">
      <div className="page">
        <SiteHeader links={[{href: "/docs", label: "Docs"}]} />
        <div className="card narrow">
          <div className="card-body">
            <h1>Your account</h1>
            <p>
              Logged in as <strong>{user.email ?? "your account"}</strong>.
            </p>
            <h2>Your live share</h2>
            {share ? (
              <p>
                <a href="/watch">Open it</a>: the latest one you started with{" "}
                <code>agent-graph watch-remote</code>. Only you can see it.
              </p>
            ) : (
              <p>
                Nothing yet. Run <code>agent-graph watch-remote</code> on your
                computer, and it&rsquo;ll be at <a href="/watch">/watch</a>.
              </p>
            )}
            <h2>Computers</h2>
            <Computers computers={computers} />
            <LogOut />
          </div>
        </div>
      </div>
    </div>
  );
}
