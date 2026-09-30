"use client";

import {useEffect, useRef, useState, useSyncExternalStore} from "react";

import type {Copied, Target} from "@/lib/analytics-core";
import {track} from "@/lib/analytics-client";
import {link} from "@/lib/links";
import {BREW_COMMAND, INSTALL_COMMAND, latestRelease} from "@/lib/release";

import {CopyCommand} from "./copy-command";

type Os = "mac" | "linux" | "windows" | "npm" | "cloud";

const release = latestRelease();

/** A command to copy, and what copying it counts as (/admin). */
type Command = {command: string; copied: Copied};

const BREW: Command = {command: BREW_COMMAND, copied: "homebrew"};
const SCRIPT: Command = {command: INSTALL_COMMAND, copied: "install-script"};
const SETUP: Command = {
  command: "agent-graph install claude-code",
  copied: "claude-code",
};
const SETUP_CODEX: Command = {
  command: "agent-graph install codex",
  copied: "codex",
};

type Way = {
  label: string;
  /** Not out yet: the tab says so, and nothing else. */
  soon?: string;
  install?: Command;
  or?: {note: string} & Command;
  /** The program itself, for each processor: counted when clicked (/admin). */
  downloads?: Array<{label: string; target: Target}>;
};

const ways: Record<Os, Way> = {
  mac: release
    ? {
        label: "macOS",
        install: BREW,
        or: {note: "Or, without Homebrew:", ...SCRIPT},
        downloads: [
          {label: "Apple silicon", target: "aarch64-apple-darwin"},
          {label: "Intel", target: "x86_64-apple-darwin"},
        ],
      }
    : {label: "macOS", soon: "The first release is coming soon."},
  linux: release
    ? {
        label: "Linux",
        install: SCRIPT,
        or: {note: "Or with Homebrew:", ...BREW},
        downloads: [
          {label: "x86_64", target: "x86_64-unknown-linux-musl"},
          {label: "ARM64", target: "aarch64-unknown-linux-musl"},
        ],
      }
    : {label: "Linux", soon: "The first release is coming soon."},
  windows: {
    label: "Windows",
    soon: "Coming soon: agent-graph for Windows, with winget.",
  },
  npm: {
    label: "npm",
    soon: "Coming soon: agent-graph from npm (npm install -g agent-graph).",
  },
  // Its steps are its own (CloudSteps).
  cloud: {label: "Claude Code cloud"},
};

/**
 * Recording and sharing live from Claude Code's cloud (claude.ai/code): the
 * project's hooks (`agent-graph install claude-code --cloud`), committed,
 * install agent-graph in each cloud session and share it to the account
 * whose API token the cloud environment gives.
 */
function CloudSteps() {
  return (
    <ol className="card-body install-steps">
      <li>
        On your computer, with agent-graph installed (the Mac or Linux tab), go
        to the root folder of your copy of the GitHub repository you&rsquo;ll
        open in the cloud, and add the cloud&rsquo;s hooks to its{" "}
        <code>.claude/settings.json</code>
        :
        <Copyable
          command="agent-graph install claude-code --cloud"
          copied="claude-code-cloud"
        />
      </li>
      <li>
        Still in that repository, commit the file and push it to the branch
        cloud sessions start from (usually <code>main</code>). A cloud session
        works in a fresh clone from GitHub, so that&rsquo;s where it finds the
        hooks. On your computer they do nothing.
        <CopyCommand
          command={
            'git add .claude/settings.json && git commit -m "Share cloud sessions to Agent Graph" && git push'
          }
        />
      </li>
      <li>
        In this site&rsquo;s{" "}
        <a href="/account#api-tokens">account page, under API tokens</a>, make a
        token, and copy it.
      </li>
      <li>
        At <a {...link("https://claude.ai/code")}>claude.ai/code</a>, open the
        environment menu beside the repository picker, and open the settings of
        the environment you&rsquo;ll use. (Not claude.ai&rsquo;s own Settings
        page: its network settings are for chats, not Claude Code.) There, add
        the token under environment variables:
        <CopyCommand command="AGENT_GRAPH_TOKEN=agt_…" />
        <span className="install-or">
          and set network access to Custom, allowing these two domains:
        </span>
        <CopyCommand
          command={"agentgraph.chofter.com\nfirebasestorage.googleapis.com"}
        />
        <span className="install-or">
          The environment&rsquo;s setup script can stay empty.
        </span>
      </li>
      <li>
        At claude.ai/code, start a session in that repository, with that
        environment. It installs agent-graph, records the session, and shares it
        live: watch it at <a href="/watch">/watch</a>.
      </li>
    </ol>
  );
}

/** A command to copy, whose copies count as downloads (/admin). */
function Copyable({command, copied}: Command) {
  return (
    <CopyCommand
      command={command}
      onCopy={() => track({event: "download", copied})}
    />
  );
}

function detectOs(): Os {
  const ua = navigator.userAgent;
  if (/Windows/i.test(ua)) {
    return "windows";
  }
  if (/Mac/i.test(ua)) {
    return "mac";
  }
  return "linux";
}

/** Marks which of the tab row's edges have tabs past them (`data-more`). */
function markMore(el: HTMLElement) {
  const left = el.scrollLeft > 1;
  const right = el.scrollLeft + el.clientWidth < el.scrollWidth - 1;
  el.dataset.more =
    left && right ? "both" : left ? "left" : right ? "right" : "";
}

export function Install() {
  // Start on the visitor's own OS; the server render uses macOS.
  const detected = useSyncExternalStore<Os>(
    () => () => {},
    detectOs,
    () => "mac",
  );
  // A tab named in the address (/#install-cloud, say: linked to from the
  // account page), until another's chosen.
  const named = useSyncExternalStore<Os | null>(
    change => {
      window.addEventListener("hashchange", change);
      return () => window.removeEventListener("hashchange", change);
    },
    () => {
      const key = location.hash.replace(/^#install-/, "");
      return key !== location.hash && key in ways ? (key as Os) : null;
    },
    () => null,
  );
  const [chosen, setOs] = useState<Os | null>(null);
  const os = chosen ?? named ?? detected;

  // On a narrow screen the tabs scroll sideways: each edge fades where
  // there's more past it (data-more), and the tab chosen is scrolled to.
  const tabs = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const el = tabs.current;
    if (!el) {
      return;
    }
    const mark = () => markMore(el);
    mark();
    el.addEventListener("scroll", mark, {passive: true});
    // (As the row's width changes: the window's, or the fonts loading.)
    const sized = new ResizeObserver(mark);
    sized.observe(el);
    return () => {
      el.removeEventListener("scroll", mark);
      sized.disconnect();
    };
  }, []);
  useEffect(() => {
    const el = tabs.current;
    const tab = el?.querySelector<HTMLElement>('[aria-selected="true"]');
    if (!el || !tab) {
      return;
    }
    // Into view within the row only (the page itself doesn't move).
    const left = tab.offsetLeft - el.offsetLeft;
    if (
      left < el.scrollLeft ||
      left + tab.offsetWidth > el.scrollLeft + el.clientWidth
    ) {
      el.scrollTo({left: left - 24});
    }
    // (Its scroll event comes a frame later: the fades are set now.)
    markMore(el);
  }, [os]);

  const way = ways[os];
  return (
    <section
      className="card install"
      id="install"
      aria-label="Install Agent Graph">
      {/* Where /#install-<tab> goes to (and opens that tab). */}
      {(Object.keys(ways) as Array<Os>).map(key => (
        <span key={key} id={`install-${key}`} className="install-anchor" />
      ))}
      <div className="tabs" role="tablist" ref={tabs}>
        {(Object.keys(ways) as Array<Os>).map(key => (
          <button
            key={key}
            type="button"
            role="tab"
            className="tab"
            aria-selected={os === key}
            onClick={() => setOs(key)}>
            {ways[key].label}
          </button>
        ))}
      </div>
      {os === "cloud" ? (
        <CloudSteps />
      ) : way.soon ? (
        <p className="card-body install-soon">{way.soon}</p>
      ) : (
        <ol className="card-body install-steps">
          <li>
            Install <code>agent-graph</code>
            {release ? (
              <span className="install-version">
                {" "}
                {release.version}
                {release.version.includes("-") ? " (beta)" : ""}
              </span>
            ) : null}
            :{way.install && <Copyable {...way.install} />}
            {way.or && (
              <>
                <span className="install-or">{way.or.note}</span>
                <Copyable {...way.or} />
              </>
            )}
            {way.downloads && (
              <>
                <span className="install-or">
                  Or download the program itself (unpack it, and put{" "}
                  <code>agent-graph</code> on your PATH):
                </span>
                <span className="downloads">
                  {way.downloads.map(d => {
                    const file = release?.files[d.target];
                    return file ? (
                      <a
                        key={d.target}
                        className="button secondary download"
                        href={file.url}
                        onClick={() =>
                          track({event: "download", target: d.target})
                        }>
                        {way.label} · {d.label}
                      </a>
                    ) : null;
                  })}
                </span>
              </>
            )}
          </li>
          <li>
            Start recording Claude Code sessions:
            <Copyable {...SETUP} />
            <span className="install-or">
              Using Codex? Record its sessions too:
            </span>
            <Copyable {...SETUP_CODEX} />
          </li>
        </ol>
      )}
    </section>
  );
}
