import "../site.css";

import type {Metadata} from "next";
import {redirect} from "next/navigation";

import {SECRET_PATTERN} from "@/lib/accounts";
import {currentUser} from "@/lib/auth";
import {safeNext} from "@/lib/config";

import {SiteHeader} from "../site-header";
import {type CliLogin, LoginForm} from "./login";

export const dynamic = "force-dynamic";

export const metadata: Metadata = {
  title: "Log in · Agent Graph",
  robots: {index: false, follow: false},
};

type Search = {next?: string; cli?: string; state?: string; challenge?: string};

/**
 * Logging in: with Google, or an email and password (lib/auth.ts). `next`
 * is where to go after. `agent-graph watch-remote` sends people here with
 * `cli` (the port it waits on), `state` and `challenge` (lib/accounts.ts):
 * once logged in, they're asked to connect it, and it's handed the login.
 */
export default async function Login({
  searchParams,
}: {
  searchParams: Promise<Search>;
}) {
  const search = await searchParams;
  const next = safeNext(search.next);
  const user = await currentUser();
  const cli = cliLogin(search);
  if (user && !search.cli) {
    redirect(next);
  }
  return (
    <div className="site">
      <div className="page">
        <SiteHeader />
        <LoginForm
          next={next}
          cli={cli}
          badCli={Boolean(search.cli) && !cli}
          email={user ? (user.email ?? "") : null}
        />
      </div>
    </div>
  );
}

function cliLogin(search: Search): CliLogin | null {
  const port = Number(search.cli);
  if (!Number.isInteger(port) || port < 1024 || port > 65535) {
    return null;
  }
  if (
    !SECRET_PATTERN.test(search.state ?? "") ||
    !SECRET_PATTERN.test(search.challenge ?? "")
  ) {
    return null;
  }
  return {port, state: search.state!, challenge: search.challenge!};
}
