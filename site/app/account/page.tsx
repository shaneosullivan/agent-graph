import "../site.css";

import type {Metadata} from "next";
import {redirect} from "next/navigation";

import {
  computersOf,
  standingOf,
  subscriptionOf,
  watchLog,
} from "@/lib/accounts";
import {currentUser} from "@/lib/auth";
import {billingConfig} from "@/lib/billing";
import {finishCheckout} from "@/lib/stripe";

import {SiteHeader} from "../site-header";
import {SiteFooter} from "../site-footer";
import {Computers, DeleteAccount, LogOut, StripeButton} from "./actions";

export const dynamic = "force-dynamic";

export const metadata: Metadata = {
  title: "Account · Agent Graph",
  robots: {index: false, follow: false},
};

/**
 * Who's logged in, their subscription (if the site isn't free:
 * lib/billing.ts), their live share, and the computers sharing to it.
 * Stripe's Checkout sends the browser back here with `checkout`, its
 * session's id, which is recorded before the page is shown.
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
  const [computers, share, sub] = await Promise.all([
    computersOf(user.uid),
    watchLog(user.uid),
    subscriptionOf(user.uid),
  ]);
  // A subscription deleting the account would cancel.
  const subscribed =
    sub !== null && !["canceled", "incomplete_expired"].includes(sub.status);
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
            <Subscription uid={user.uid} />
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
            <h2 id="delete" className="delete-heading">
              Delete your account
            </h2>
            <p>
              Deletes your account, your live shares and your computers&rsquo;
              logins, and cancels any subscription.
            </p>
            <DeleteAccount email={user.email} subscribed={subscribed} />
          </div>
        </div>
        <SiteFooter />
      </div>
    </div>
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

/** Where the account stands, and a way to subscribe, or manage it. */
async function Subscription({uid}: {uid: string}) {
  const billing = billingConfig();
  if (!billing) {
    return null;
  }
  const [{status, canShare, freeUntil}, sub] = await Promise.all([
    standingOf(uid),
    subscriptionOf(uid),
  ]);
  const {monthly, yearly} = billing.plans;
  const price = `${monthly.label}, or ${yearly.label}`;
  const testMode =
    billing.mode === "test" ? (
      <p className="test-mode">
        Stripe is in test mode: nothing&rsquo;s charged. Pay with the card 4242
        4242 4242 4242, any future date and any CVC.
      </p>
    ) : null;
  const manage = sub ? (
    <StripeButton path="/api/account/billing" secondary>
      Manage billing
    </StripeButton>
  ) : null;

  if (status === "active") {
    const end = sub?.periodEnd ?? null;
    return (
      <>
        <h2>Subscription</h2>
        {testMode}
        <p>
          You&rsquo;re subscribed
          {sub?.plan
            ? `, ${sub.plan}, for ${billing.plans[sub.plan].label}`
            : ""}
          .{" "}
          {sub?.cancelAt
            ? `It ends on ${date(sub.cancelAt)}.`
            : end && sub?.status === "trialing"
              ? `Your first payment is on ${date(end)}.`
              : end
                ? `It renews on ${date(end)}.`
                : null}
        </p>
        <p>{manage}</p>
      </>
    );
  }
  return (
    <>
      <h2>Subscription</h2>
      {testMode}
      {canShare && freeUntil !== null ? (
        <p>
          Sharing live is free until {date(freeUntil)}. After that it&rsquo;s{" "}
          {price}. Subscribe now, and you&rsquo;re charged only once the free
          days are over.
        </p>
      ) : (
        <p className="error">
          {sub
            ? "Your subscription has ended."
            : `Your ${billing.freeDays} free days are over.`}{" "}
          Subscribe, for {price}, to keep sharing live. A watch-remote left
          running carries on by itself once you have.
        </p>
      )}
      <p>
        <StripeButton path="/api/account/subscribe" body={{plan: "yearly"}}>
          Subscribe yearly: {yearly.label}
        </StripeButton>{" "}
        <StripeButton
          path="/api/account/subscribe"
          body={{plan: "monthly"}}
          secondary>
          Subscribe monthly: {monthly.label}
        </StripeButton>{" "}
        {manage}
      </p>
    </>
  );
}
