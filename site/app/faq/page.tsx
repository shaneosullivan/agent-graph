import "../site.css";

import type {Metadata} from "next";

import {billingConfig} from "@/lib/billing";
import {CONTACT} from "@/lib/contact";
import {link} from "@/lib/links";

import {InfoPage} from "../info-page";

// The prices and free days are the site's settings now (lib/billing.ts).
export const dynamic = "force-dynamic";

export const metadata: Metadata = {
  title: "FAQ · Agent Graph",
  description:
    "Is Agent Graph free? What does sharing live cost? Which platforms does it run on? Answers to common questions.",
};

export default function Faq() {
  const billing = billingConfig();
  const days = billing?.freeDays ?? 0;
  const trial =
    days === 7 ? "first week" : `first ${days} day${days === 1 ? "" : "s"}`;
  return (
    <InfoPage title="Frequently asked questions">
      <h2>Is Agent Graph open source?</h2>
      <p>
        Yes. The <code>agent-graph</code> command and this site are open source,
        under the MIT licence:{" "}
        <a {...link(CONTACT.source)}>
          {CONTACT.source.replace("https://", "")}
        </a>
        . You can read the code, change it, and use it for anything, including
        at work.
      </p>

      <h2>Is it free?</h2>
      <p>
        Using it on your own computer is free, for ever: recording your
        agents&rsquo; sessions, and <code>agent-graph view</code> to see them in
        your browser. Nothing leaves your computer, and there&rsquo;s no account
        to make.
      </p>
      <p>
        Sharing a log, by pasting or uploading it here, is free too. Only
        sharing live to your account (<code>agent-graph watch-remote</code>, to
        follow your agents from your phone, say) may cost something: see below.
      </p>

      <h2>What does sharing live cost?</h2>
      {billing ? (
        <>
          <p>
            It&rsquo;s free for your {trial}. After that, it&rsquo;s{" "}
            <strong>{billing.plans.monthly.label}</strong>, or{" "}
            <strong>{billing.plans.yearly.label}</strong>. You can cancel any
            time, from your account page.
          </p>
          <p>
            That&rsquo;s to cover what running the site costs: storing and
            serving everyone&rsquo;s live shares as their agents work.
            Everything else stays free.
          </p>
        </>
      ) : (
        <p>
          Nothing, at the moment. If that changes, the price will be shown here,
          and before you&rsquo;re ever asked to pay; it would be only to cover
          what running the site costs.
        </p>
      )}

      <h2>Can I share live without paying?</h2>
      <p>
        Yes: run the site yourself. It&rsquo;s the same code as this one, and
        runs on Vercel and Firebase, whose free plans should be enough for one
        person. Without Stripe&rsquo;s settings, it doesn&rsquo;t charge at all.
        Then point <code>agent-graph watch-remote --url</code> at your own
        address. The{" "}
        <a {...link(`${CONTACT.source}/tree/main/site`)}>site&rsquo;s README</a>{" "}
        says how to set it up.
      </p>

      <h2>Which platforms does it run on?</h2>
      <p>
        For now, <strong>macOS</strong> (Apple silicon and Intel) and{" "}
        <strong>Linux</strong> (x86_64 and ARM64), with{" "}
        <strong>Claude Code</strong> and <strong>Codex</strong>: it records
        their sessions, and the agents they start, on your computer and in their
        clouds (claude.ai/code and chatgpt.com/codex). We&rsquo;re still testing
        it on the other platforms and coding agents, so Windows, installing from
        npm, and recording Cursor are coming, but not ready yet.{" "}
        <a href="/#install">Install it</a> with Homebrew, or the install script.
      </p>

      <h2>What does it record?</h2>
      <p>
        By default, short labels: task names, what agents were asked to do, and
        summaries of messages between them. Not your prompts, or what tools
        return. The <a href="/privacy">Privacy</a> page says more.
      </p>

      <h2>Something else?</h2>
      <p>
        Ask us: <a href="/contact">get in touch</a>.
      </p>
    </InfoPage>
  );
}
