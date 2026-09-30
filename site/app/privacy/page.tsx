import "../site.css";

import type {Metadata} from "next";

import {CONTACT} from "@/lib/contact";
import {link} from "@/lib/links";

import {InfoPage} from "../info-page";

export const metadata: Metadata = {
  title: "Privacy · Agent Graph",
  description:
    "What Agent Graph records, what this site stores, who can see it, and how long it's kept.",
};

// What's said here is what the code does: keep it true when that changes
// (lib/accounts.ts, lib/store.ts, lib/encryption.ts, lib/analytics*.ts,
// lib/auth.ts, lib/billing.ts, lib/cleanup.ts).
export default function Privacy() {
  return (
    <InfoPage title="Privacy" updated="30 September 2026">
      <p>
        Agent Graph is built to keep what it records to a minimum, and to keep
        it on your computer unless you choose to share it. This page says what
        the <code>agent-graph</code> command records, what this site
        (agentgraph.chofter.com) stores, who can see it, and how long it&rsquo;s
        kept. The site is run by {CONTACT.company}, in {CONTACT.country}, which
        is the controller of the personal data it holds. For questions, email{" "}
        <a href={`mailto:${CONTACT.email}`}>{CONTACT.email}</a>.
      </p>

      <h2>On your computer</h2>
      <p>
        The <code>agent-graph</code> command records your AI coding
        agents&rsquo; sessions in <code>~/.agent-graph</code> on your own
        computer. By default it keeps short labels (task names, agent
        descriptions, summaries of messages between agents), not your prompts,
        what tools return, or agents&rsquo; replies, unless you turn that on
        with <code>AGENT_GRAPH_CAPTURE_BODIES=1</code>. Recording sends nothing
        anywhere, and <code>agent-graph view</code> shows it on your computer
        only. Nothing reaches this site unless you paste or upload a log, or run{" "}
        <code>agent-graph watch-remote</code>.
      </p>

      <h2>Logs you share</h2>
      <ul>
        <li>
          <strong>Pasted or uploaded logs</strong> can be opened by anyone who
          has their link, unless you set a password when you share one; then
          they need that too. The password is stored only as a scrypt hash.
        </li>
        <li>
          <strong>Live shares</strong> (<code>agent-graph watch-remote</code>)
          belong to your account, and only you, logged in, can see them.
        </li>
        <li>
          Every log is stored encrypted (AES-256-GCM, with a key of its own),
          under a name derived from its link, so the database holds neither the
          link nor the log in the clear.
        </li>
        <li>
          A log that has had no new events for a week is deleted, by a job that
          runs every day. A live share keeps only its most recent events (the
          site deletes older ones as new ones arrive).
        </li>
      </ul>

      <h2>Your account</h2>
      <p>
        You need an account only to share live. You log in with Google, or an
        email address and password, through Firebase Authentication (Google),
        which holds your login. This site stores, for your account:
      </p>
      <ul>
        <li>
          your email address, when the account was made, and when you last
          logged in and last shared live;
        </li>
        <li>which live share is your latest (encrypted);</li>
        <li>
          each computer you&rsquo;ve logged in from with{" "}
          <code>agent-graph watch-remote</code>: its name (the computer&rsquo;s
          hostname), when it logged in and was last used, and a hash of its
          login (never the login itself). You can log any of them out on your
          account page;
        </li>
        <li>
          whether you&rsquo;re subscribed, if the site charges for sharing live
          (below).
        </li>
      </ul>
      <p>
        While <code>watch-remote</code> is running it tells the site so every
        minute or so: which computer it&rsquo;s on (its name), and a summary of
        each session it&rsquo;s watching (its name, folder and state), so the
        home page and <a href="/watch">/watch</a> can show them, from all your
        computers together. Like your logs, it&rsquo;s stored encrypted.
      </p>

      <h2>Payments</h2>
      <p>
        Subscriptions are handled by Stripe. Your card details go to Stripe,
        never to this site. The site gives Stripe your email address and an
        account id, and keeps Stripe&rsquo;s customer id, and your
        subscription&rsquo;s plan, status and renewal or end date.
        Stripe&rsquo;s own privacy policy covers what it does with your payment
        details.
      </p>

      <h2>Cookies and counting visits</h2>
      <ul>
        <li>
          Logged in, the site sets a session cookie (<code>__session</code>, for
          14 days) and a cookie that just says you&rsquo;re logged in, so pages
          can show the right links. Having opened a log with a password, a
          cookie remembers that for that log (for 30 days).
        </li>
        <li>
          The site counts page views, visitors, downloads, sign-ups and live
          shares, as totals per day and per month, in its own database. It uses
          no analytics service and no tracking cookies, and stores no IP
          address, browser details or identifier with them: a visitor is counted
          using your browser&rsquo;s own storage, which holds only the date of
          your last visit. Daily totals are deleted after a year.
        </li>
        <li>
          To limit password guessing, the site counts wrong guesses for a short
          while, by a keyed hash of your network address, not the address
          itself.
        </li>
      </ul>

      <h2>Who else handles it</h2>
      <ul>
        <li>Vercel hosts the site.</li>
        <li>
          Google (Firebase and Google Cloud) holds accounts, logs and the
          site&rsquo;s database, and serves downloads of the command.
        </li>
        <li>Stripe takes payments.</li>
        <li>
          The home page shows a video from YouTube (Google), from its
          privacy-enhanced domain: it sets no cookies until you play the video
          yourself, but your browser does fetch it from YouTube.
        </li>
      </ul>
      <p>
        We don&rsquo;t sell your data, share it with advertisers, or use it to
        train models. The site&rsquo;s own admin pages show only totals (the
        counts above), and we don&rsquo;t look at your logs or live shares.
      </p>

      <h2>Your choices</h2>
      <ul>
        <li>Don&rsquo;t share: everything stays on your computer.</li>
        <li>
          Log a computer out on your account page, or with{" "}
          <code>agent-graph watch-remote --logout</code>.
        </li>
        <li>
          Stop sharing live, and your share is deleted after a week without new
          events.
        </li>
        <li>Cancel a subscription from your account page.</li>
        <li>
          Delete your account, with your live shares and your computers&rsquo;
          logins, on your <a href="/account#delete">account page</a>. It cancels
          any subscription too.
        </li>
        <li>
          To ask what we hold about you, email{" "}
          <a href={`mailto:${CONTACT.email}`}>{CONTACT.email}</a> from the
          address you log in with.
        </li>
      </ul>

      <h2>Your rights</h2>
      <p>
        Under data protection law (the GDPR) you can ask for a copy of the
        personal data we hold about you, have it corrected or deleted, object to
        or restrict how it&rsquo;s used, and take it elsewhere. We use it to
        provide the service you&rsquo;ve asked for (your account, your live
        shares, your subscription), to keep the site secure, and to count its
        use in aggregate. Email{" "}
        <a href={`mailto:${CONTACT.email}`}>{CONTACT.email}</a>, and we&rsquo;ll
        reply within a week. If you&rsquo;re not happy with how we handle it,
        you can complain to Ireland&rsquo;s Data Protection Commission, at{" "}
        <a {...link("https://www.dataprotection.ie")}>dataprotection.ie</a>, or
        the authority where you live.
      </p>
      <p>
        The services above may process data outside the European Economic Area
        (in the United States, say); where they do, it&rsquo;s under the
        safeguards data protection law requires, such as the European
        Commission&rsquo;s standard contractual clauses.
      </p>

      <h2>Changes</h2>
      <p>
        If this changes, we&rsquo;ll update this page and the date at the top.
        Agent Graph is open source, so you can always check what it does:{" "}
        <a {...link(CONTACT.source)}>
          {CONTACT.source.replace("https://", "")}
        </a>
        .
      </p>
    </InfoPage>
  );
}
