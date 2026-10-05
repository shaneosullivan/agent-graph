import "../site.css";

import type {Metadata} from "next";

import {Inline} from "@/lib/help-render";
import troubleshooting from "@/lib/troubleshooting.json";

import {InfoPage} from "../info-page";

// What can go wrong, and how to fix it: lib/troubleshooting.json is copied
// from docs/troubleshooting.json, which `agent-graph diagnostics` has too,
// and links to here, at #<id>.
type Issue = {
  id: string;
  title: string;
  problem: string;
  fix: string;
  clouds?: Record<string, string>;
};
const issues = (troubleshooting as unknown as {issues: Array<Issue>}).issues;

export const metadata: Metadata = {
  title: "Troubleshooting · Agent Graph",
  description:
    "Sessions not recorded, or not shared? What can go wrong with Agent Graph, on your computer and in a coding agent's cloud, and how to fix it.",
};

export default function Troubleshooting() {
  return (
    <InfoPage title="Troubleshooting">
      <p>
        Start with <code>agent-graph diagnostics</code>, wherever sessions
        aren&rsquo;t recorded or shared: on your computer, or in a cloud
        agent&rsquo;s machine (ask the agent to run{" "}
        <code>~/.local/bin/agent-graph diagnostics</code>). It checks everything
        Agent Graph needs there, names what&rsquo;s wrong, and links to the fix
        below. Nothing secret is printed.
      </p>
      <p>
        A cloud that&rsquo;s set up to share reports its own problems: they show
        at the top of <a href="/watch">/watch</a>.
      </p>
      <nav aria-label="Problems">
        <ul>
          {issues.map(i => (
            <li key={i.id}>
              <a href={`#${i.id}`}>{i.title}</a>
            </li>
          ))}
        </ul>
      </nav>
      {issues.map(i => (
        <section key={i.id} id={i.id} className="trouble">
          <h2>
            <a href={`#${i.id}`} className="anchor">
              {i.title}
            </a>
          </h2>
          <p>
            <Inline text={i.problem} />
          </p>
          <p>
            <strong>Fix:</strong> <Inline text={i.fix} />
          </p>
          {i.clouds && Object.keys(i.clouds).length > 0 ? (
            <ul>
              {Object.entries(i.clouds).map(([cloud, fix]) => (
                <li key={cloud}>
                  <strong>In {cloud}:</strong> <Inline text={fix} />
                </li>
              ))}
            </ul>
          ) : null}
        </section>
      ))}
    </InfoPage>
  );
}
