import "../site.css";

import type {Metadata} from "next";

import {CONTACT} from "@/lib/contact";

import {InfoPage} from "../info-page";

export const metadata: Metadata = {
  title: "About · Agent Graph",
  description:
    "Agent Graph shows what your AI coding agents are doing: which started which, who's waiting on whom, and what needs you.",
};

export default function About() {
  return (
    <InfoPage title="About Agent Graph">
      <p>
        When you&rsquo;re running several AI coding agents at once (a Claude
        Code session that starts explorers and planners, which start workers of
        their own), it gets hard to see what&rsquo;s happening. Agent Graph
        records what your agents do, as they do it, and draws it: which session
        started which agents, who&rsquo;s waiting on whom, what looks stuck, and
        what needs you.
      </p>
      <h2>How it works</h2>
      <ul>
        <li>
          The <code>agent-graph</code> command records your agents&rsquo;
          sessions on your own computer, through Claude Code&rsquo;s hooks. By
          default it records task names and agent descriptions, not your prompts
          or what tools return.
        </li>
        <li>
          <code>agent-graph view</code> opens a viewer on your computer, in your
          browser. Nothing leaves your computer to use it.
        </li>
        <li>
          This site is for sharing: paste or upload a log to get a link, or run{" "}
          <code>agent-graph watch-remote</code> to follow your own sessions live
          from anywhere, your phone included, logged in to your account.
        </li>
      </ul>
      <h2>Open source</h2>
      <p>
        Agent Graph is open source, under the MIT licence:{" "}
        <a href={CONTACT.source}>{CONTACT.source.replace("https://", "")}</a>.
        You can run everything yourself, this site included.
      </p>
      <h2>Who makes it</h2>
      <p>
        Agent Graph is made by Shane O&rsquo;Sullivan, at {CONTACT.company} in{" "}
        {CONTACT.country}. Say hello on X at{" "}
        <a href={CONTACT.twitter.url}>{CONTACT.twitter.handle}</a>, on Threads
        at <a href={CONTACT.threads.url}>{CONTACT.threads.handle}</a>, or by{" "}
        <a href="/contact">email</a>.
      </p>
    </InfoPage>
  );
}
