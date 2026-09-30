import "../site.css";

import type {Metadata} from "next";

import {CONTACT} from "@/lib/contact";

import {InfoPage} from "../info-page";

export const metadata: Metadata = {
  title: "Terms & Conditions · Agent Graph",
  description: "The terms of using the Agent Graph site and its live sharing.",
};

export default function Terms() {
  return (
    <InfoPage title="Terms & Conditions" updated="30 September 2026">
      <p>
        These terms cover your use of this site, agentgraph.chofter.com (the
        &ldquo;site&rdquo;), run by {CONTACT.company}, in {CONTACT.country}{" "}
        (&ldquo;we&rdquo;): sharing logs, and sharing live with an account. By
        using the site you agree to them. If you don&rsquo;t, please don&rsquo;t
        use it. How we handle data is in the <a href="/privacy">Privacy</a>{" "}
        page, which is part of these terms.
      </p>

      <h2>The software</h2>
      <p>
        The <code>agent-graph</code> command, and this site&rsquo;s code, are
        open source, under the MIT licence:{" "}
        <a href={CONTACT.source}>{CONTACT.source.replace("https://", "")}</a>.
        That licence, not these terms, covers the software itself: you&rsquo;re
        free to use it, change it, and run your own copy of the site.
      </p>

      <h2>What you share</h2>
      <ul>
        <li>
          What you paste, upload or share live stays yours. You give us only the
          permission we need to store it, and show it to whoever you share it
          with, for as long as the site keeps it.
        </li>
        <li>
          You&rsquo;re responsible for what you share, and for having the right
          to share it. Don&rsquo;t share anything unlawful, anything that
          infringes someone else&rsquo;s rights, or secrets (passwords, keys,
          personal data) you wouldn&rsquo;t want seen.
        </li>
        <li>
          A pasted or uploaded log can be opened by anyone with its link, unless
          you set a password. Keep links you care about private.
        </li>
        <li>
          Logs are deleted after a week without new events, and a live share
          keeps only its recent events. The site isn&rsquo;t a backup: keep your
          own copy of anything you need.
        </li>
      </ul>

      <h2>Your account</h2>
      <ul>
        <li>
          Keep your login to yourself, and log out computers you no longer use
          (on your account page). You&rsquo;re responsible for what&rsquo;s done
          with your account.
        </li>
        <li>
          We may suspend or close an account, or remove a log, that breaks these
          terms or puts the site or others at risk.
        </li>
      </ul>

      <h2>Subscriptions</h2>
      <ul>
        <li>
          Sharing live may need a subscription after a free trial. The price,
          and how long the trial is, are shown before you subscribe.
        </li>
        <li>
          Stripe takes payment, and a subscription renews each month or year
          until you cancel it. You can cancel any time from your account page;
          it runs to the end of the period you&rsquo;ve paid for, and then
          stops. Payments aren&rsquo;t refunded for part of a period, except
          where the law says they must be.
        </li>
        <li>
          If a payment fails or a subscription ends, sharing live stops (after a
          few days&rsquo; grace) until you subscribe again. Everything else on
          the site stays free.
        </li>
        <li>
          If prices change, we&rsquo;ll tell subscribers before they&rsquo;re
          charged the new price.
        </li>
      </ul>

      <h2>Using the site fairly</h2>
      <p>
        Don&rsquo;t try to break into the site, get at others&rsquo; logs or
        accounts, get round its limits, or overload it.
      </p>

      <h2>No warranty</h2>
      <p>
        The site is provided as it is, without warranties of any kind. We work
        to keep it running and your data safe, but can&rsquo;t promise it will
        always be available, free of errors, or that data won&rsquo;t be lost.
        As far as the law allows, we aren&rsquo;t liable for indirect or
        consequential losses from using it, and our total liability to you is
        limited to what you&rsquo;ve paid us in the twelve months before the
        claim. Nothing here limits rights you have by law that can&rsquo;t be
        limited.
      </p>

      <h2>Changes</h2>
      <p>
        We may change these terms, or the site. If a change matters, we&rsquo;ll
        update the date at the top and, for subscribers, tell you by email
        before it applies. Using the site after that means you accept the new
        terms.
      </p>

      <h2>The law</h2>
      <p>
        These terms are governed by the laws of Ireland, and the courts of
        Ireland deal with any dispute about them. If you use the site as a
        consumer in another country, you keep the protections its laws give you,
        and can bring a claim in its courts.
      </p>

      <h2>Contact</h2>
      <p>
        Questions about these terms:{" "}
        <a href={`mailto:${CONTACT.email}`}>{CONTACT.email}</a>.
      </p>
    </InfoPage>
  );
}
