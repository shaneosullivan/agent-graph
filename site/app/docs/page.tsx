import "../site.css";

import type {Metadata} from "next";

import cliHelp from "@/lib/cli-help.json";
import {Blocks, Examples, type Help, Inline, Options} from "@/lib/help-render";

import {GettingStarted, GettingStartedLinks} from "../install";
import {SiteHeader} from "../site-header";
import {SiteFooter} from "../site-footer";

// The same text `agent-graph --help` prints: lib/cli-help.json is copied from
// docs/cli-help.json (which build.rs compiles into the binary) by `npm run dev`
// and `npm run build`.
const help = cliHelp as unknown as Help;

export const metadata: Metadata = {
  title: "Docs · Agent Graph",
  description: `How to use the ${help.program} command: ${help.summary}`,
};

/**
 * The page's contents: its two parts, the getting-started platforms, the
 * reference's sections and its commands. Beside the page on a wide screen;
 * on a phone, in the header's menu (SiteMenu), not above the page.
 */
function DocsContents() {
  return (
    <>
      <a href="#getting-started" className="docs-nav-part">
        Getting started
      </a>
      <GettingStartedLinks />
      <a href="#reference" className="docs-nav-part">
        Reference
      </a>
      <a href="#overview">Overview</a>
      {help.sections.map(s => (
        <a key={s.title} href={`#${slug(titled(s.title))}`}>
          {titled(s.title)}
        </a>
      ))}
      <span className="docs-nav-label">Commands</span>
      {help.commands.map(c => (
        <a key={c.name} href={`#${c.name}`}>
          <code>{c.name}</code>
        </a>
      ))}
    </>
  );
}

export default function Docs() {
  return (
    <div className="site">
      <div className="page docs">
        <SiteHeader
          links={[
            {href: "/", label: "Share a log"},
            {href: "/docs", label: "Docs"},
          ]}
          menu={<DocsContents />}
        />

        <div className="docs-layout">
          <aside className="docs-nav" aria-label="Contents">
            <DocsContents />
          </aside>

          <main className="docs-main">
            <section id="getting-started" className="hero">
              <h1>Getting started</h1>
              <p>
                Install <code>{help.program}</code> and start recording your
                agents&rsquo; sessions. Choose where they run:
              </p>
            </section>
            <GettingStarted />

            <section id="reference" className="hero docs-part">
              <h1>Reference</h1>
            </section>
            <section id="overview" className="doc-section">
              <h2>
                The <code>{help.program}</code> command
              </h2>
              <p>{help.summary}</p>
            </section>
            <div className="prose">
              <Blocks blocks={help.description} />
            </div>

            {help.sections.map(section => (
              <section
                key={section.title}
                id={slug(titled(section.title))}
                className="doc-section">
                <h2>{titled(section.title)}</h2>
                {section.blocks && (
                  <div className="prose">
                    <Blocks blocks={section.blocks} />
                  </div>
                )}
                {section.examples && <Examples examples={section.examples} />}
              </section>
            ))}

            <h2 className="commands-title">Commands</h2>
            <p className="muted">
              <Inline text={help.footer} />
            </p>
            {help.commands.map(command => (
              <section
                key={command.name}
                id={command.name}
                className="doc-command">
                <h3>
                  <code>
                    {help.program} {command.name}
                  </code>
                </h3>
                <p className="command-summary">
                  <Inline text={command.summary} />
                </p>
                <div className="prose">
                  <Blocks blocks={command.description} />
                </div>
                <h4>Options</h4>
                <Options options={command.options} />
                <h4>Examples</h4>
                <Examples examples={command.examples} />
              </section>
            ))}
          </main>
        </div>
        <SiteFooter />
      </div>
    </div>
  );
}

function slug(title: string): string {
  return title
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "");
}

/**
 * A section of `--help`, as the docs name it: its "Getting started" (the
 * commands to run first) is "First steps" here, where Getting started is
 * the page's own first part, installing.
 */
function titled(title: string): string {
  return title === "Getting started" ? "First steps" : title;
}
