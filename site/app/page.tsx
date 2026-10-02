import "./site.css";

import {Fragment} from "react";

import cliHelp from "@/lib/cli-help.json";
import {type Help, Inline} from "@/lib/help-render";
import {hasWindows, latestRelease} from "@/lib/release";

import {DeletedNotice} from "./deleted-notice";
import {Examples} from "./examples";
import {Install} from "./install";
import {SiteHeader} from "./site-header";
import {SiteFooter} from "./site-footer";
import {Uploader} from "./uploader";
import {WatchCard} from "./watch-card";
import {YouTubeVideo} from "./youtube-video";

const watchRemote = (cliHelp as unknown as Help).commands.find(
  c => c.name === "watch-remote",
)!;

export default function Home() {
  return (
    <div className="site">
      <div className="page">
        <SiteHeader
          links={[
            {href: "#install", label: "Install"},
            {href: "#examples", label: "Examples"},
            {href: "#live", label: "Share live"},
            {href: "/docs", label: "Docs"},
          ]}
        />

        <section className="hero">
          <h1>Lead your army of agents with confidence</h1>

          <p>
            We humans are still an integral cog in the AI machine, but when
            we're running dozens (or even just 4 or 5) agents or sessions, we
            often become the bottleneck. An agent is waiting on our input, a
            session is blocked by a child session it spawned, and unless we
            monitor them like a hawk, they remain blocked.
          </p>
          <p>
            <strong>Agent Graph</strong> solves this.
          </p>
        </section>

        <DeletedNotice />

        <WatchCard
          loggedOutContent={
            <section>
              <h2>How does Agent Graph help?</h2>
              <p>
                Agent Graph integrates with leading AI coding providers like{" "}
                <a
                  href="https://claude.com/product/claude-code"
                  target="_blank"
                  rel="noopener noreferrer">
                  Claude Code
                </a>{" "}
                and{" "}
                <a
                  href="https://openai.com/codex/"
                  target="_blank"
                  rel="noopener noreferrer">
                  Codex
                </a>{" "}
                to record a log of all your sessions, agents and tasks and the
                dependencies between them. It highlights where you are needed at
                any time, what work is ongoing, and what is completed. You can
                run it completely locally, it starts a local web server that you
                use on your computer.
              </p>
              <YouTubeVideo
                id="v0GHik_RKJM"
                title="Agent Graph, in action"
                autoplay
              />
              <h3>What about when I'm away from my computer?</h3>
              <p>
                To keep up with your agents on the move, you simply run{" "}
                <code>agent-graph watch-remote</code>, and that streams your
                logs to this site, where you can track all your agent's work in
                a fully secure and private manner. This is completely optional,
                and you can of course keep all of your logs local on your
                machine. This entire project is open source too of course, so
                you can even fork it, deploy this site yourself to your own
                domain, and use it for your work instead of this site.
              </p>
            </section>
          }
        />

        <section className="section">
          <h2>Install Agent Graph</h2>
          <p className="section-lead">
            {hasWindows(latestRelease())
              ? "For macOS, Linux and Windows, on ARM and x86_64. "
              : "For macOS and Linux, on ARM and x86_64 (Windows is coming). "}
            It records Claude Code and Codex, on your computer and in their
            clouds.
          </p>
          <Install />
        </section>

        <Examples />

        <section className="section">
          <h2>Share your logs</h2>

          <p>
            Paste or upload an Agent Graph log to get a link, get it from{" "}
            <code>.agent-graph/events</code> in your home folder. It opens a
            viewer you can step through: which session started which agents,
            who&rsquo;s waiting on whom, what looks stuck, and what needs you.
            You can share the url with a teammate and it'll stay live for a
            week.
          </p>
          <Uploader />
        </section>

        <section className="section" id="live">
          <h2>Share live from your machine</h2>
          <p>
            With the <code>agent-graph</code> command installed, this shares to
            your account (logging you in the first time) and keeps it up to date
            as your agents work. Only you can see it, at{" "}
            <a href="/watch">/watch</a>:
          </p>
          <code className="command">agent-graph watch-remote</code>
          {/* The same text as `agent-graph watch-remote --help`. */}
          <dl className="options">
            {watchRemote.options.map(option =>
              // (Not --url, for another site, nor --background, which
              // --autostart's service runs it with.)
              option.id !== "url" && option.id !== "background" ? (
                <Fragment key={option.id}>
                  <dt>
                    <code>{option.flag}</code>
                  </dt>
                  <dd>
                    <Inline text={option.text} />
                  </dd>
                </Fragment>
              ) : null,
            )}
          </dl>
          <p>
            <a href="/docs#watch-remote">More about sharing</a>, and every other
            command, in the docs.
          </p>
        </section>

        <section className="section">
          <h2>What the viewer shows</h2>
          <div className="grid3">
            <div className="tile">
              <h3>
                <span className="dot" style={{background: "var(--warn)"}} />
                Needs you
              </h3>
              <p>
                Permission prompts, questions and plans waiting for your
                approval.
              </p>
            </div>
            <div className="tile">
              <h3>
                <span className="dot" style={{background: "var(--bad)"}} />
                Stuck or deadlocked
              </h3>
              <p>
                Agents that went quiet while working, and sessions waiting on
                each other.
              </p>
            </div>
            <div className="tile">
              <h3>
                <span className="dot" style={{background: "var(--accent)"}} />A
                timeline
              </h3>
              <p>
                Step through every event and see the graph exactly as it was at
                that moment.
              </p>
            </div>
          </div>
        </section>

        <section className="section" id="logs">
          <h2>Where logs live</h2>
          <p>
            Agent Graph records one file per session in{" "}
            <code>~/.agent-graph/events/</code> (on Windows,{" "}
            <code>%USERPROFILE%\.agent-graph\events\</code>). Upload any of
            them, or paste their contents.
          </p>
        </section>

        <SiteFooter>
          A pasted or uploaded log can be viewed by anyone with its link, unless
          you set a password; a live share is only yours. By default Agent Graph
          records task names and agent descriptions, not prompts or tool output.
        </SiteFooter>
      </div>
    </div>
  );
}
