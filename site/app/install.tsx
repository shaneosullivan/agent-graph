"use client";

import {useState, useSyncExternalStore} from "react";

import type {Copied, Target} from "@/lib/analytics-core";
import {track} from "@/lib/analytics-client";
import {BREW_COMMAND, INSTALL_COMMAND, latestRelease} from "@/lib/release";

import {CopyCommand} from "./copy-command";

type Os = "mac" | "linux" | "windows" | "npm";

const release = latestRelease();

/** A command to copy, and what copying it counts as (/admin). */
type Command = {command: string; copied: Copied};

const BREW: Command = {command: BREW_COMMAND, copied: "homebrew"};
const SCRIPT: Command = {command: INSTALL_COMMAND, copied: "install-script"};
const SETUP: Command = {
  command: "agent-graph install claude-code",
  copied: "claude-code",
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
};

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

export function Install() {
  // Start on the visitor's own OS; the server render uses macOS.
  const detected = useSyncExternalStore<Os>(
    () => () => {},
    detectOs,
    () => "mac",
  );
  const [chosen, setOs] = useState<Os | null>(null);
  const os = chosen ?? detected;

  const way = ways[os];
  return (
    <section
      className="card install"
      id="install"
      aria-label="Install Agent Graph">
      <div className="tabs" role="tablist">
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
      {way.soon ? (
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
          </li>
        </ol>
      )}
    </section>
  );
}
