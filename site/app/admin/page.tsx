import "../site.css";
import "./admin.css";

import type {Metadata} from "next";
import {notFound} from "next/navigation";

import {activeWatches, lastDays, lastMonths} from "@/lib/analytics";
import {analyticsDisabled, isAdmin} from "@/lib/analytics-core";
import {currentUser} from "@/lib/auth";

import {SiteHeader} from "../site-header";
import {Dashboard} from "./dashboard";

export const dynamic = "force-dynamic";

export const metadata: Metadata = {
  title: "Admin · Agent Graph",
  robots: {index: false, follow: false},
};

/**
 * How the site's used (lib/analytics.ts), for the accounts in ADMIN_EMAILS
 * (lib/analytics-core.ts, `isAdmin`); anyone else is told there's no such
 * page.
 */
export default async function Admin() {
  const user = await currentUser();
  if (!isAdmin(user)) {
    notFound();
  }
  const [days, months, active] = await Promise.all([
    lastDays(30),
    lastMonths(12),
    activeWatches(),
  ]);
  return (
    <div className="site">
      <div className="page wide">
        <SiteHeader links={[{href: "/docs", label: "Docs"}]} />
        <h1 className="admin-title">Usage</h1>
        <Dashboard
          days={days}
          months={months}
          active={active}
          disabled={analyticsDisabled()}
        />
      </div>
    </div>
  );
}
