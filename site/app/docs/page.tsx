import "../site.css";

import type { Metadata } from "next";

import cliHelp from "@/lib/cli-help.json";
import { Blocks, Examples, type Help, Inline, Options } from "@/lib/help-render";

import { SiteHeader } from "../site-header";

// The same text `agent-graph --help` prints: lib/cli-help.json is copied from
// docs/cli-help.json (which build.rs compiles into the binary) by `npm run dev`
// and `npm run build`.
const help = cliHelp as unknown as Help;

export const metadata: Metadata = {
  title: "Docs · Agent Graph",
  description: `How to use the ${help.program} command: ${help.summary}`,
};

export default function Docs() {
  return (
    <div className="site">
      <div className="page docs">
        <SiteHeader
          links={[
            { href: "/", label: "Share a log" },
            { href: "/docs", label: "Docs" },
          ]}
        />

        <div className="docs-layout">
          <aside className="docs-nav" aria-label="Commands">
            <a href="#overview">Overview</a>
            {help.sections.map((s) => (
              <a key={s.title} href={`#${slug(s.title)}`}>
                {s.title}
              </a>
            ))}
            <span className="docs-nav-label">Commands</span>
            {help.commands.map((c) => (
              <a key={c.name} href={`#${c.name}`}>
                <code>{c.name}</code>
              </a>
            ))}
          </aside>

          <main className="docs-main">
            <section id="overview" className="hero">
              <h1>
                The <code>{help.program}</code> command
              </h1>
              <p>{help.summary}</p>
            </section>
            <div className="prose">
              <Blocks blocks={help.description} />
            </div>

            {help.sections.map((section) => (
              <section key={section.title} id={slug(section.title)} className="doc-section">
                <h2>{section.title}</h2>
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
            {help.commands.map((command) => (
              <section key={command.name} id={command.name} className="doc-command">
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
