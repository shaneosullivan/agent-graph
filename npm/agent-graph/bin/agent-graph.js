#!/usr/bin/env node
// Runs agent-graph's program for this machine. It's in its own package,
// @chofter/agent-graph-<platform>-<arch>, one of this package's optional
// dependencies: npm installs only the one whose os and cpu match.
"use strict";

const {spawn} = require("child_process");
const path = require("path");

const platform = `${process.platform}-${process.arch}`;
const pkg = `@chofter/agent-graph-${platform}`;
const exe = process.platform === "win32" ? "agent-graph.exe" : "agent-graph";

let bin;
try {
  bin = require.resolve(`${pkg}/bin/${exe}`);
} catch {
  const supported = Object.keys(
    require(path.join(__dirname, "..", "package.json")).optionalDependencies ||
      {},
  ).map(name => name.replace("@chofter/agent-graph-", ""));
  console.error(
    supported.includes(platform)
      ? `agent-graph: ${pkg} isn't installed. It's an optional dependency, ` +
          `so it's left out by --omit=optional or --no-optional: ` +
          `install @chofter/agent-graph again without that.`
      : `agent-graph: there's no build for ${platform} yet ` +
          `(there are for ${supported.join(", ")}). ` +
          `See https://agentgraph.chofter.com/#install`,
  );
  process.exit(1);
}

const child = spawn(bin, process.argv.slice(2), {stdio: "inherit"});

// The terminal sends Ctrl+C to both: the program decides what it means
// (`tail` quits, `run` passes it on), and this waits for it to exit; passing
// it on as well would send it twice. A SIGTERM or SIGHUP is passed on.
const signals = ["SIGINT", "SIGTERM", "SIGHUP"];
const forward = signal => () => {
  if (signal !== "SIGINT") child.kill(signal);
};
const handlers = signals.map(signal => [signal, forward(signal)]);
for (const [signal, handler] of handlers) process.on(signal, handler);

child.on("error", err => {
  console.error(`agent-graph: can't run ${bin}: ${err.message}`);
  process.exit(1);
});
child.on("exit", (code, signal) => {
  for (const [s, handler] of handlers) process.off(s, handler);
  if (signal) {
    // Killed by a signal: so is this, so whatever ran it sees the same.
    process.kill(process.pid, signal);
  } else {
    process.exit(code ?? 1);
  }
});
