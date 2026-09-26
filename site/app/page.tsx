import "./site.css";

import { Fragment } from "react";

import cliHelp from "@/lib/cli-help.json";
import { type Help, Inline } from "@/lib/help-render";

import { Brand } from "./brand";
import { Uploader } from "./uploader";

const watchRemote = (cliHelp as unknown as Help).commands.find((c) => c.name === "watch-remote")!;

export default function Home() {
  return (
    <div className="site">
      <div className="page">
        <header className="top">
          <Brand />
          <nav>
            <a href="#live">Share live</a>
            <a href="/docs">Docs</a>
          </nav>
        </header>

        <section className="hero">
          <h1>See what your AI coding agents are doing, and share it.</h1>
          <p>
            Paste or upload an Agent Graph log to get a link. It opens a viewer you can step through: which
            session started which agents, who&rsquo;s waiting on whom, what looks stuck, and what needs you.
            A log is deleted once it has had no new events for a week.
          </p>
        </section>

        <Uploader />

        <section className="section" id="live">
          <h2>Share live from your machine</h2>
          <p>
            With the <code>agent-graph</code> command installed, this prints a link straight away and keeps
            the page up to date as your agents work:
          </p>
          <code className="command">agent-graph watch-remote</code>
          {/* The same text as `agent-graph watch-remote --help`. */}
          <dl className="options">
            {watchRemote.options.map((option) => (
              <Fragment key={option.id}>
                <dt>
                  <code>{option.flag}</code>
                </dt>
                <dd>
                  <Inline text={option.text} />
                </dd>
              </Fragment>
            ))}
          </dl>
          <p>
            <a href="/docs#watch-remote">More about sharing</a>, and every other command, in the docs.
          </p>
        </section>

        <section className="section">
          <h2>What the viewer shows</h2>
          <div className="grid3">
            <div className="tile">
              <h3>
                <span className="dot" style={{ background: "var(--warn)" }} />
                Needs you
              </h3>
              <p>Permission prompts, questions and plans waiting for your approval.</p>
            </div>
            <div className="tile">
              <h3>
                <span className="dot" style={{ background: "var(--bad)" }} />
                Stuck or deadlocked
              </h3>
              <p>Agents that went quiet while working, and sessions waiting on each other.</p>
            </div>
            <div className="tile">
              <h3>
                <span className="dot" style={{ background: "var(--accent)" }} />A timeline
              </h3>
              <p>Step through every event and see the graph exactly as it was at that moment.</p>
            </div>
          </div>
        </section>

        <section className="section" id="logs">
          <h2>Where logs live</h2>
          <p>
            Agent Graph records one file per session in <code>~/.agent-graph/events/</code> (on Windows,{" "}
            <code>%USERPROFILE%\.agent-graph\events\</code>). Upload any of them, or paste their contents.
          </p>
        </section>

        <footer className="foot">
          Shared logs are stored so that anyone with the link can view them, unless you set a password. By
          default Agent Graph records task names and agent descriptions, not prompts or tool output.
        </footer>
      </div>
    </div>
  );
}
