import "../site.css";

import type {Metadata} from "next";
import {redirect} from "next/navigation";

import {
  computersOf,
  standingOf,
  type Subscription as Sub,
  subscriptionOf,
} from "@/lib/accounts";
import {keysOf} from "@/lib/api/keys";
import {currentUser} from "@/lib/auth";
import {breakagesOf} from "@/lib/cloud-diagnostics";
import {type Billing, billingConfig, type Standing} from "@/lib/billing";
import {type Share, sharesOf} from "@/lib/store";
import {finishCheckout} from "@/lib/stripe";

import {CopyCommand} from "../copy-command";
import {SiteFooter} from "../site-footer";
import {SiteHeader} from "../site-header";
import {AccountNav} from "./account-nav";
import {ApiKeys} from "./api-keys";
import {
  ApiTokens,
  Computers,
  DeleteAccount,
  HiddenCloudProblems,
  LogOut,
  StripeButton,
} from "./actions";
import {
  AlertIcon,
  ArrowIcon,
  CardIcon,
  KeyIcon,
  LaptopIcon,
  LiveIcon,
} from "./icons";

export const dynamic = "force-dynamic";

export const metadata: Metadata = {
  title: "Account · Agent Graph",
  robots: {index: false, follow: false},
};

/** How long after its last word a share's still taken to be running. */
const ALIVE_FOR_MS = 3 * 60 * 1000;

/**
 * The account: at a glance (its plan, what it's sharing, its computers and
 * API tokens), then a panel for each, and last, deleting it. Stripe's
 * Checkout sends the browser back here with `checkout`, its session's id,
 * which is recorded before the page is shown.
 */
export default async function Account({
  searchParams,
}: {
  searchParams: Promise<{checkout?: string}>;
}) {
  const user = await currentUser();
  if (!user) {
    redirect("/login?next=/account");
  }
  const {checkout} = await searchParams;
  if (typeof checkout === "string") {
    await finishCheckout(user.uid, checkout).catch(() => {});
    redirect("/account");
  }
  const billing = billingConfig();
  const {computers, shares, keys, sub, standing, hidden, now} = await load(
    user.uid,
  );
  const machines = computers.filter(c => c.kind === "computer");
  const tokens = computers.filter(c => c.kind === "api");
  const running = shares.filter(s => now - s.at <= ALIVE_FOR_MS);
  const sessionsNow = running.reduce((n, s) => n + s.sessions, 0);
  // A subscription deleting the account would cancel.
  const subscribed =
    sub !== null && !["canceled", "incomplete_expired"].includes(sub.status);
  const plan = planSummary(billing, standing, sub, now);

  return (
    <div className="site">
      <div className="page page-wide">
        <SiteHeader links={[{href: "/docs", label: "Docs"}]} />
        <div className="acct">
          <header className="acct-head">
            <span className="acct-avatar" aria-hidden="true">
              {(user.email ?? "?").slice(0, 1).toUpperCase()}
            </span>
            <div className="acct-who">
              <h1>Account</h1>
              <p>{user.email ?? "Logged in"}</p>
            </div>
            <LogOut />
          </header>

          <div className="acct-layout">
            <AccountNav />

            <div className="acct-main">
              <section
                id="overview"
                className="acct-tiles"
                aria-label="At a glance">
                <a href="#billing" className={`acct-tile ${plan.tone}`}>
                  <span className="acct-tile-label">
                    <CardIcon /> Plan
                  </span>
                  <strong>{plan.title}</strong>
                  <span className="acct-tile-sub">{plan.sub}</span>
                </a>
                <a
                  href="#live-sharing"
                  className={`acct-tile ${running.length ? "good" : ""}`}>
                  <span className="acct-tile-label">
                    <LiveIcon /> Live sharing
                  </span>
                  <strong>
                    {running.length ? (
                      <>
                        <span className="acct-live-dot" /> Sharing now
                      </>
                    ) : (
                      "Not sharing"
                    )}
                  </strong>
                  <span className="acct-tile-sub">
                    {running.length
                      ? `${plural(sessionsNow, "session")} on ${plural(running.length, "computer")}`
                      : shares.length
                        ? `Last shared ${ago(shares[0].at, now)}`
                        : "Nothing shared yet"}
                  </span>
                </a>
                <a href="#computers" className="acct-tile">
                  <span className="acct-tile-label">
                    <LaptopIcon /> Computers
                  </span>
                  <strong>{machines.length}</strong>
                  <span className="acct-tile-sub">logged in to share</span>
                </a>
                <a href="#api-tokens" className="acct-tile">
                  <span className="acct-tile-label">
                    <KeyIcon /> API tokens
                  </span>
                  <strong>{tokens.length}</strong>
                  <span className="acct-tile-sub">for cloud machines</span>
                </a>
              </section>

              <Panel
                id="billing"
                icon={<CardIcon />}
                title="Billing"
                description="Sharing live costs a little, to cover what running the site costs. Everything else is free.">
                <BillingPanel
                  billing={billing}
                  standing={standing}
                  sub={sub}
                  plan={plan}
                />
              </Panel>

              <Panel
                id="live-sharing"
                icon={<LiveIcon />}
                title="Live sharing"
                description="What your computers and cloud machines are sharing, which only you can see."
                action={
                  shares.length ? (
                    <a className="button acct-action" href="/watch">
                      Open live view <ArrowIcon />
                    </a>
                  ) : null
                }>
                <LivePanel shares={shares} now={now} />
              </Panel>

              <Panel
                id="computers"
                icon={<LaptopIcon />}
                title="Computers"
                description="Where agent-graph is logged in to share to this account. Removing one logs it out there.">
                <Computers computers={machines} />
              </Panel>

              <Panel
                id="api-tokens"
                icon={<KeyIcon />}
                title="API tokens"
                description="For a machine where you can't log in in a browser, such as Claude Code's cloud. Keep them secret: anyone with one can share to this account.">
                <ApiTokens tokens={tokens} />
              </Panel>

              <Panel
                id="api-keys"
                icon={<KeyIcon />}
                title="API keys"
                description="Read-only keys to the graph API, for your own servers and the AI systems they run: they read your live shares' graphs, and nothing else. Keep them secret.">
                <ApiKeys
                  keys={keys.map(k => ({
                    id: k.id,
                    name: k.name,
                    kind: k.kind,
                    graphs: k.graphs?.map(g => `gph_${g}`) ?? null,
                    shown: k.shown,
                    createdAt: k.createdAt,
                    usedAt: k.usedAt,
                  }))}
                  shares={shares.map(s => ({id: s.id, host: s.host}))}
                />
              </Panel>

              {hidden.length ? (
                <Panel
                  id="cloud-problems"
                  icon={<AlertIcon />}
                  title="Hidden cloud problems"
                  description="Problems a cloud reported that you chose never to be shown again, at /watch and in agent-graph view.">
                  <HiddenCloudProblems
                    hidden={hidden.map(h => ({
                      key: h.key,
                      cloud: h.cloud,
                      issues: h.issues,
                      at: h.at,
                    }))}
                  />
                </Panel>
              ) : null}

              <Panel
                id="delete"
                icon={<AlertIcon />}
                title="Delete account"
                description="Deletes your account, your live shares and your computers' logins, and cancels any subscription. It can't be undone."
                danger>
                <DeleteAccount email={user.email} subscribed={subscribed} />
              </Panel>
            </div>
          </div>
        </div>
        <SiteFooter />
      </div>
    </div>
  );
}

/** What the page shows of account `uid`, as of now (and when that is). */
async function load(uid: string) {
  const [computers, shares, keys, sub, standing, hidden] = await Promise.all([
    computersOf(uid),
    sharesOf(uid),
    keysOf(uid),
    subscriptionOf(uid),
    standingOf(uid),
    breakagesOf(uid, true),
  ]);
  return {computers, shares, keys, sub, standing, hidden, now: Date.now()};
}

/** One of the account's panels: its title and what it's for, then its body. */
function Panel({
  id,
  icon,
  title,
  description,
  action,
  danger,
  children,
}: {
  id: string;
  icon: React.ReactNode;
  title: string;
  description: string;
  action?: React.ReactNode;
  danger?: boolean;
  children: React.ReactNode;
}) {
  return (
    <section
      id={id}
      className={`acct-panel${danger ? " danger" : ""}`}
      aria-labelledby={`${id}-title`}>
      <div className="acct-panel-head">
        <span className="acct-panel-icon">{icon}</span>
        <div className="acct-panel-title">
          <h2 id={`${id}-title`}>{title}</h2>
          <p>{description}</p>
        </div>
        {action}
      </div>
      <div className="acct-panel-body">{children}</div>
    </section>
  );
}

// The same on the server and in the browser, whatever either's locale.
const date = (ms: number) =>
  new Date(ms).toLocaleDateString("en-GB", {
    day: "numeric",
    month: "long",
    year: "numeric",
    timeZone: "UTC",
  });

function plural(n: number, what: string): string {
  return `${n} ${what}${n === 1 ? "" : "s"}`;
}

/** How long ago `ms` was, roughly: "just now", "5 minutes ago", "3 days ago". */
function ago(ms: number, now: number): string {
  const minutes = Math.round((now - ms) / 60000);
  if (minutes < 1) {
    return "just now";
  }
  if (minutes < 60) {
    return `${plural(minutes, "minute")} ago`;
  }
  const hours = Math.round(minutes / 60);
  if (hours < 24) {
    return `${plural(hours, "hour")} ago`;
  }
  return `${plural(Math.round(hours / 24), "day")} ago`;
}

/** The plan, as the overview and the billing panel say it. */
type PlanSummary = {
  title: string;
  sub: string;
  /** How it's shown: well, needing attention, or neither. */
  tone: "" | "good" | "warn";
  badge: string;
};

function planSummary(
  billing: Billing | null,
  {status, canShare, freeUntil}: Standing,
  sub: Sub | null,
  now: number,
): PlanSummary {
  if (!billing) {
    return {
      title: "Free",
      sub: "Sharing live is free",
      tone: "good",
      badge: "Free",
    };
  }
  if (status === "active") {
    const name = sub?.plan ? capitalise(sub.plan) : "Subscribed";
    if (sub?.cancelAt) {
      return {
        title: name,
        sub: `Ends ${date(sub.cancelAt)}`,
        tone: "warn",
        badge: "Cancelling",
      };
    }
    if (sub?.status === "trialing" && sub.periodEnd) {
      return {
        title: name,
        sub: `First payment ${date(sub.periodEnd)}`,
        tone: "good",
        badge: "Trial",
      };
    }
    return {
      title: name,
      sub: sub?.periodEnd ? `Renews ${date(sub.periodEnd)}` : "Active",
      tone: "good",
      badge: "Active",
    };
  }
  if (canShare && freeUntil !== null) {
    const days = Math.max(1, Math.ceil((freeUntil - now) / 86400000));
    return {
      title: "Free trial",
      sub: `${plural(days, "day")} left`,
      tone: "",
      badge: "Trial",
    };
  }
  return {
    title: sub ? "Ended" : "Trial over",
    sub: "Subscribe to share live",
    tone: "warn",
    badge: sub ? "Ended" : "Trial over",
  };
}

function capitalise(s: string): string {
  return s.slice(0, 1).toUpperCase() + s.slice(1);
}

/** Where the account stands, and a way to subscribe, or manage it. */
function BillingPanel({
  billing,
  standing,
  sub,
  plan,
}: {
  billing: Billing | null;
  standing: Standing;
  sub: Sub | null;
  plan: PlanSummary;
}) {
  if (!billing) {
    return (
      <p className="acct-muted">
        Sharing live is free on this site: there&rsquo;s nothing to pay.
      </p>
    );
  }
  const {monthly, yearly} = billing.plans;
  const manage = sub ? (
    <StripeButton path="/api/account/billing" secondary>
      Manage billing
    </StripeButton>
  ) : null;
  const says =
    standing.status === "active"
      ? `You're subscribed${sub?.plan ? `, ${sub.plan}, for ${billing.plans[sub.plan].label}` : ""}. ${plan.sub}.`
      : standing.canShare && standing.freeUntil !== null
        ? `Sharing live is free until ${date(standing.freeUntil)}. Subscribe now, and you're charged only once the free days are over.`
        : `${sub ? "Your subscription has ended." : `Your ${billing.freeDays} free days are over.`} Subscribe to keep sharing live: a watch-remote left running carries on by itself once you have.`;
  const status = (
    <div className="acct-status">
      <span className={`acct-badge ${plan.tone}`}>{plan.badge}</span>
      <span>{says}</span>
    </div>
  );
  const testMode =
    billing.mode === "test" ? (
      <p className="acct-note">
        Stripe is in test mode: nothing&rsquo;s charged. Pay with the card 4242
        4242 4242 4242, any future date and any CVC.
      </p>
    ) : null;

  if (standing.status === "active") {
    return (
      <>
        {testMode}
        {status}
        {manage ? <div className="acct-actions">{manage}</div> : null}
      </>
    );
  }
  return (
    <>
      {testMode}
      {status}
      <div className="acct-plans">
        <div className="acct-plan featured">
          <span className="acct-plan-tag">Best value</span>
          <h3>Yearly</h3>
          <p className="acct-plan-price">{yearly.label}</p>
          <StripeButton path="/api/account/subscribe" body={{plan: "yearly"}}>
            Subscribe yearly
          </StripeButton>
        </div>
        <div className="acct-plan">
          <h3>Monthly</h3>
          <p className="acct-plan-price">{monthly.label}</p>
          <StripeButton
            path="/api/account/subscribe"
            body={{plan: "monthly"}}
            secondary>
            Subscribe monthly
          </StripeButton>
        </div>
      </div>
      {manage ? <div className="acct-actions">{manage}</div> : null}
    </>
  );
}

/** The account's live shares: each computer, sharing now or not, and its sessions. */
function LivePanel({shares, now}: {shares: Array<Share>; now: number}) {
  if (!shares.length) {
    return (
      <div className="acct-empty">
        <span className="acct-empty-icon">
          <LiveIcon />
        </span>
        <p>
          <strong>Nothing shared yet.</strong> Run this on your computer, and
          what your agents are doing shows at <a href="/watch">/watch</a>:
        </p>
        <CopyCommand command="agent-graph watch-remote" />
        <p className="acct-muted">
          Or share from Claude Code&rsquo;s cloud:{" "}
          <a href="/#install-cloud">see how</a>.
        </p>
      </div>
    );
  }
  return (
    <ul className="acct-rows">
      {shares.map(share => {
        const live = now - share.at <= ALIVE_FOR_MS;
        return (
          <li key={share.id}>
            <span className={`acct-row-icon${live ? " live" : ""}`}>
              <LaptopIcon />
            </span>
            <span className="acct-row-main">
              <strong>{share.host || "A computer"}</strong>
              <span className="acct-row-sub">
                {plural(share.sessions, "session")} ·{" "}
                {live ? "sharing now" : `last shared ${ago(share.at, now)}`}
              </span>
            </span>
            <span className={`acct-badge ${live ? "good" : ""}`}>
              {live ? (
                <>
                  <span className="acct-live-dot" /> Live
                </>
              ) : (
                "Stopped"
              )}
            </span>
          </li>
        );
      })}
    </ul>
  );
}
