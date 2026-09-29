"use client";

import {useState, useSyncExternalStore} from "react";

type Os = "mac" | "linux" | "windows" | "npm";

const RELEASES =
  "https://github.com/shaneosullivan/agent-graph/releases/latest/download";

const ways: Record<
  Os,
  {label: string; install: string; or?: {note: string; command: string}}
> = {
  mac: {
    label: "macOS",
    install: "brew install shaneosullivan/tap/agent-graph",
    or: {
      note: "Or, without Homebrew:",
      command: `curl --proto '=https' --tlsv1.2 -LsSf ${RELEASES}/agent-graph-installer.sh | sh`,
    },
  },
  linux: {
    label: "Linux",
    install: `curl --proto '=https' --tlsv1.2 -LsSf ${RELEASES}/agent-graph-installer.sh | sh`,
    or: {
      note: "Or with Homebrew:",
      command: "brew install shaneosullivan/tap/agent-graph",
    },
  },
  windows: {
    label: "Windows",
    install: "winget install ShaneOSullivan.AgentGraph",
    or: {
      note: "Or in PowerShell:",
      command: `powershell -ExecutionPolicy Bypass -c "irm ${RELEASES}/agent-graph-installer.ps1 | iex"`,
    },
  },
  npm: {
    label: "npm",
    install: "npm install -g agent-graph",
  },
};

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
      <ol className="card-body install-steps">
        <li>
          Install <code>agent-graph</code>:
          <code className="command">{way.install}</code>
          {way.or && (
            <>
              <span className="install-or">{way.or.note}</span>
              <code className="command">{way.or.command}</code>
            </>
          )}
        </li>
        <li>
          Start recording Claude Code sessions:
          <code className="command">agent-graph install claude-code</code>
        </li>
      </ol>
    </section>
  );
}
