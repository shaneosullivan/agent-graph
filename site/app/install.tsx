"use client";

import {
  Fragment,
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";

import type {Copied, Target} from "@/lib/analytics-core";
import {track} from "@/lib/analytics-client";
import {link} from "@/lib/links";
import {
  BREW_COMMAND,
  hasCursor,
  hasWindows,
  INSTALL_COMMAND,
  latestRelease,
  POWERSHELL_COMMAND,
  WGET_INSTALL_COMMAND,
} from "@/lib/release";

import {CopyCommand} from "./copy-command";

type Os =
  | "mac"
  | "linux"
  | "windows"
  | "npm"
  | "cloud"
  | "codex-cloud"
  | "cursor-cloud";

const release = latestRelease();

/** A command to copy, and what copying it counts as (/admin). */
type Command = {command: string; copied: Copied};

const BREW: Command = {command: BREW_COMMAND, copied: "homebrew"};
const SCRIPT: Command = {command: INSTALL_COMMAND, copied: "install-script"};
const WGET_SCRIPT: Command = {
  command: WGET_INSTALL_COMMAND,
  copied: "install-script",
};
const POWERSHELL: Command = {command: POWERSHELL_COMMAND, copied: "powershell"};
// Global, so the hooks' path to the program stays (npx's doesn't).
const NPM: Command = {
  command: "npm install -g @chofter/agent-graph",
  copied: "npm",
};
const SETUP: Command = {
  command: "agent-graph install claude-code",
  copied: "claude-code",
};
const SETUP_CODEX: Command = {
  command: "agent-graph install codex",
  copied: "codex",
};
const SETUP_CURSOR: Command = {
  command: "agent-graph install cursor",
  copied: "cursor",
};

type Way = {
  label: string;
  /** Not out yet: the tab says so, and nothing else. */
  soon?: string;
  install?: Command;
  /** Said under the install command. */
  note?: string;
  /** Other ways to install it, each with what's said above it. */
  or?: Array<{note: string} & Command>;
  /** The program itself, for each processor: counted when clicked (/admin). */
  downloads?: Array<{label: string; target: Target}>;
};

const ways: Record<Os, Way> = {
  mac: release
    ? {
        label: "macOS",
        install: BREW,
        or: [{note: "Or, without Homebrew:", ...SCRIPT}],
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
        or: [
          {note: "No curl? With wget:", ...WGET_SCRIPT},
          {note: "Or with Homebrew:", ...BREW},
        ],
        downloads: [
          {label: "x86_64", target: "x86_64-unknown-linux-musl"},
          {label: "ARM64", target: "aarch64-unknown-linux-musl"},
        ],
      }
    : {label: "Linux", soon: "The first release is coming soon."},
  windows: hasWindows(release)
    ? {
        label: "Windows",
        install: POWERSHELL,
        note: "In PowerShell. It puts agent-graph.exe in %USERPROFILE%\\.local\\bin, and adds that to your PATH. Run it again to upgrade.",
        ...(release?.npm ? {or: [{note: "Or with Node (npm):", ...NPM}]} : {}),
        downloads: [
          {label: "x64", target: "x86_64-pc-windows-msvc"},
          {label: "ARM64", target: "aarch64-pc-windows-msvc"},
        ],
      }
    : {
        label: "Windows",
        soon: "Coming soon: agent-graph for Windows.",
      },
  npm: release?.npm
    ? {
        label: "npm",
        install: NPM,
        note: `For macOS${hasWindows(release) ? ", Linux and Windows" : " and Linux"}, with Node 16 or later. Install it globally, not with npx: the hooks run the program from where npm puts it. npm update -g @chofter/agent-graph upgrades it.`,
      }
    : {
        label: "npm",
        soon: "Coming soon: agent-graph from npm (npm install -g @chofter/agent-graph).",
      },
  // Its steps are its own (CloudSteps).
  cloud: {label: "Claude Code cloud"},
  // Its steps are its own too (CodexCloudSteps).
  "codex-cloud": {label: "Codex cloud"},
  // And so are its (CursorCloudSteps).
  "cursor-cloud": hasCursor(release)
    ? {label: "Cursor cloud"}
    : {
        label: "Cursor cloud",
        soon: "Coming soon: recording and sharing Cursor's cloud agents.",
      },
};

/**
 * What goes in a Codex cloud environment's setup script. The install
 * script's address is different every time (the time, as `cache_bust`), so
 * no cache on the way gives an older one, with an older release.
 */
const CODEX_CLOUD_SETUP = `curl -fsSL "https://agentgraph.chofter.com/install.sh?cache_bust=$(date +%s)" | sh
~/.local/bin/agent-graph install codex --cloud --yes`;

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
        On your computer, with agent-graph installed (as for macOS or Linux), go
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

/**
 * Recording and sharing live from Codex's cloud (chatgpt.com/codex): the
 * environment's setup script installs agent-graph and its hooks
 * (`agent-graph install codex --cloud`), and each task shares to the account
 * whose API token the environment gives. Every step is done on a web page:
 * nothing is installed on the visitor's computer, or committed.
 */
function CodexCloudSteps() {
  return (
    <ol className="card-body install-steps">
      <li>
        On this site&rsquo;s{" "}
        <a href="/account#api-tokens">account page, under API tokens</a>, make a
        token, and copy it (it starts with <code>agt_</code>). It&rsquo;s shown
        only once, so keep the page open until step 4.
      </li>
      <li>
        Go to <a {...link("https://chatgpt.com/codex")}>chatgpt.com/codex</a>{" "}
        and sign in with your ChatGPT account. If you see a page that says
        &ldquo;Build anything with Codex&rdquo;, click{" "}
        <strong>Go to Cloud</strong> at its top right.
      </li>
      <li>
        Click the gear icon at the top right (beside the bell and your picture),
        then <strong>Environments</strong>. Click the environment for the GitHub
        repository you&rsquo;ll work in (create one for it there first, if there
        isn&rsquo;t one), then click <strong>Edit</strong>. Everything in steps
        4 to 7 is on that Edit page. (Not ChatGPT&rsquo;s own settings, whose
        options are for chats.)
        <span className="install-or">
          Each environment has its own settings: only tasks in an environment
          set up this way are recorded. If you have more than one, do steps 3 to
          7 for each of them (step 9 says so again).
        </span>
      </li>
      <li>
        Under <strong>Environment variables</strong>, add one named{" "}
        <code>AGENT_GRAPH_TOKEN</code>, with the token from step 1 as its value:
        <CopyCommand command="AGENT_GRAPH_TOKEN" />
        <span className="install-or">
          Not under <strong>Secrets</strong>: a secret reaches only the setup
          script, not the task, so nothing would be shared.
        </span>
      </li>
      <li>
        Under <strong>Setup script</strong>, choose <strong>Manual</strong>, and
        add these two lines to the box, after anything already there:
        <Copyable command={CODEX_CLOUD_SETUP} copied="codex-cloud" />
        <span className="install-or">
          They install agent-graph in the cloud&rsquo;s machine (not on your
          computer), and the hooks that record each task there and share it.
        </span>
      </li>
      <li>
        Under <strong>Agent internet access</strong>, choose <strong>On</strong>
        . Keep the <strong>Domain allowlist</strong> as it is, and in{" "}
        <strong>Additional allowed domains</strong> add (after a comma, if there
        are domains there already):
        <CopyCommand command="agentgraph.chofter.com" />
        <span className="install-or">
          Set <strong>Allowed HTTP Methods</strong> to{" "}
          <strong>All methods</strong>: sharing sends the session with POST
          requests.
        </span>
      </li>
      <li>
        Click <strong>Save environment</strong>, at the bottom right.
      </li>
      <li>
        Back on the environment&rsquo;s page, click <strong>Use this</strong>,
        and give Codex a task. Its session is recorded and shared live: watch it
        at <a href="/watch">/watch</a>, where it&rsquo;s labelled &ldquo;Codex
        cloud&rdquo;. Every task in that environment is shared the same way,
        with nothing more to do.
        <span className="install-or">
          Nothing at /watch? On the environment&rsquo;s page, click{" "}
          <strong>Reset cache</strong>, so the setup script runs again, then
          start a new task. Codex can read the token, as it can any environment
          variable, but it can only share to your account; delete it on your
          account page to stop it working.
        </span>
      </li>
      <li>
        Using other environments too, ones you already had or new ones? Each
        needs the same settings: go back to <strong>Environments</strong>, and
        do steps 3 to 7 for each of them. You can use the same token in all of
        them. Tasks in an environment without them aren&rsquo;t recorded.
      </li>
    </ol>
  );
}

/**
 * Recording and sharing live from Cursor's cloud agents: the project's
 * Cursor hooks (`agent-graph install cursor --cloud --scope project`),
 * committed, install agent-graph on each cloud agent's machine and share it
 * to the account whose API token is a secret in Cursor's dashboard.
 */
function CursorCloudSteps() {
  return (
    <ol className="card-body install-steps">
      <li>
        On your computer, with agent-graph installed (as for macOS or Linux), go
        to the root folder of your copy of the GitHub repository Cursor&rsquo;s
        cloud agents will work in, and add the cloud&rsquo;s hooks to its{" "}
        <code>.cursor/hooks.json</code>:
        <Copyable
          command="agent-graph install cursor --cloud --scope project"
          copied="cursor-cloud"
        />
      </li>
      <li>
        Still in that repository, commit the file and push it to the branch
        cloud agents start from (usually <code>main</code>). A cloud agent works
        in a clone from GitHub, so that&rsquo;s where it finds the hooks. On
        your computer, and your teammates&rsquo;, they do nothing.
        <CopyCommand
          command={
            'git add .cursor/hooks.json && git commit -m "Share Cursor cloud agents to Agent Graph" && git push'
          }
        />
      </li>
      <li>
        In this site&rsquo;s{" "}
        <a href="/account#api-tokens">account page, under API tokens</a>, make a
        token, and copy it (it starts with <code>agt_</code>).
      </li>
      <li>
        At <a {...link("https://cursor.com/dashboard")}>cursor.com/dashboard</a>
        , open <strong>Cloud Agents</strong>, then <strong>Secrets</strong>, and
        add a secret named <code>AGENT_GRAPH_TOKEN</code>, with the token as its
        value:
        <CopyCommand command="AGENT_GRAPH_TOKEN" />
      </li>
      <li>
        Still in Cursor, open the settings of the cloud environment for that
        repository, and set its <strong>Start Script</strong> to:
        <CopyCommand command="git pull --ff-only" />
        <span className="install-or">
          A cloud agent starts from the environment&rsquo;s saved copy of the
          repository, which can be older than your latest push: without the
          hooks, if they were committed after the environment was made. This
          brings it up to date first. The hooks install agent-graph themselves,
          so the Install Script can stay as it is.
        </span>
      </li>
      <li>
        Start a cloud agent on that repository (at{" "}
        <a {...link("https://cursor.com/agents")}>cursor.com/agents</a>, or from
        the Cursor app). Its hooks install agent-graph on its machine, then
        record the agent and its subagents and share them live: watch them at{" "}
        <a href="/watch">/watch</a>, where they&rsquo;re labelled &ldquo;Cursor
        cloud&rdquo;, with the names Cursor gives them.
        <span className="install-or">
          Nothing at /watch? Ask the agent to run{" "}
          <code>~/.local/bin/agent-graph diagnostics</code>, which also says
          whether it started behind its branch, or see{" "}
          <a href="/troubleshooting">Troubleshooting</a>.
        </span>
        <span className="install-or">
          A cloud agent can read the token, as it can any secret, but it can
          only share to your account; delete it on your account page to stop it
          working.
        </span>
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
            id={`install-tab-${key}`}
            type="button"
            role="tab"
            className="tab"
            aria-selected={os === key}
            aria-controls={`install-panel-${key}`}
            onClick={() => setOs(key)}>
            {ways[key].label}
          </button>
        ))}
      </div>
      {/* Every tab's steps are in the page, for search engines; only the
          chosen one is shown. */}
      {(Object.keys(ways) as Array<Os>).map(key => (
        <div
          key={key}
          id={`install-panel-${key}`}
          role="tabpanel"
          aria-labelledby={`install-tab-${key}`}
          hidden={os !== key}>
          <PlatformSteps os={key} />
        </div>
      ))}
    </section>
  );
}

/**
 * The docs' Getting started: every platform's steps, as the Download
 * section's tabs have them, one after another, with a list of them first.
 * Each is at /docs#get-started-<platform>.
 */
export function GettingStarted() {
  const platforms = Object.keys(ways) as Array<Os>;
  return (
    <>
      <nav className="get-started-toc" aria-label="Platforms">
        <ul>
          {platforms.map(os => (
            <li key={os}>
              <a href={`#get-started-${os}`}>{ways[os].label}</a>
            </li>
          ))}
        </ul>
      </nav>
      {platforms.map(os => (
        <section
          key={os}
          id={`get-started-${os}`}
          className="doc-section get-started">
          <h3>{ways[os].label}</h3>
          <div className="card install">
            <PlatformSteps os={os} />
          </div>
        </section>
      ))}
    </>
  );
}

/** The platforms the docs list, with their names: for its menu. */
export function GettingStartedLinks() {
  return (
    <>
      {(Object.keys(ways) as Array<Os>).map(os => (
        <a key={os} href={`#get-started-${os}`}>
          {ways[os].label}
        </a>
      ))}
    </>
  );
}

/**
 * How to install Agent Graph on platform `os`, and start recording: the
 * Download section's tab for it, and the docs' Getting started.
 */
export function PlatformSteps({os}: {os: Os}) {
  const way = ways[os];
  return (
    <>
      {os === "cloud" ? (
        <CloudSteps />
      ) : os === "codex-cloud" ? (
        <CodexCloudSteps />
      ) : os === "cursor-cloud" && !way.soon ? (
        <CursorCloudSteps />
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
            {way.note && <span className="install-or">{way.note}</span>}
            {way.or?.map(or => (
              <Fragment key={or.command}>
                <span className="install-or">{or.note}</span>
                <Copyable {...or} />
              </Fragment>
            ))}
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
            {hasCursor(release) && (
              <>
                <span className="install-or">
                  Using Cursor? Record its chats, in the app and its CLI:
                </span>
                <Copyable {...SETUP_CURSOR} />
              </>
            )}
          </li>
        </ol>
      )}
    </>
  );
}
