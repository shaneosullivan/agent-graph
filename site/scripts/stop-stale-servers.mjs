// Stops site servers an earlier CI run left running: `next start` from one
// of this repository's checkouts (scripts/ci-api-test.sh starts it, and a
// cancelled run's keeps running on Windows, where cancelling doesn't stop
// it). Other processes, other projects' servers included, are left alone.
//
//   node scripts/stop-stale-servers.mjs
//
// Only under CI (CI=true): on a developer's machine, a server like that is
// theirs.

import {spawnSync} from "node:child_process";

if (process.env.CI !== "true") {
  process.exit(0);
}

const windows = process.platform === "win32";

/** Every node process: its id and command line. */
function nodeProcesses() {
  if (windows) {
    const ps = spawnSync(
      "powershell.exe",
      [
        "-NoProfile",
        "-Command",
        "Get-CimInstance Win32_Process -Filter \"Name = 'node.exe'\" | Select-Object ProcessId, CommandLine | ConvertTo-Json -Compress",
      ],
      {encoding: "utf8"},
    );
    if (ps.status !== 0 || !ps.stdout.trim()) {
      return [];
    }
    const found = JSON.parse(ps.stdout);
    return (Array.isArray(found) ? found : [found]).map(p => ({
      pid: p.ProcessId,
      command: p.CommandLine ?? "",
    }));
  }
  const ps = spawnSync("ps", ["-axo", "pid=,command="], {encoding: "utf8"});
  return ps.stdout
    .split("\n")
    .map(line => line.trim().match(/^(\d+)\s+(.*)$/))
    .filter(Boolean)
    .map(([, pid, command]) => ({pid: Number(pid), command}));
}

/** A command line's words, without the quotes around any. */
const words = command =>
  (command.match(/"[^"]*"|\S+/g) ?? []).map(w => w.replace(/^"|"$/g, ""));

// This repository's site's next, run by node with `start`: node, then
// .../agent-graph/.../site/node_modules/next/..., in any run's checkout.
const stale = nodeProcesses().filter(({pid, command}) => {
  const [program, script, ...rest] = words(command);
  return (
    pid !== process.pid &&
    /(^|[\\/])node(\.exe)?$/i.test(program ?? "") &&
    /[\\/]agent-graph[\\/](.*[\\/])?site[\\/]node_modules[\\/]next[\\/]/i.test(
      script ?? "",
    ) &&
    rest.includes("start")
  );
});

for (const {pid, command} of stale) {
  console.log(`Stopping a site server an earlier run left: ${pid} ${command}`);
  if (windows) {
    // Its child processes too.
    spawnSync("taskkill", ["/PID", String(pid), "/T", "/F"], {
      stdio: "inherit",
    });
  } else {
    try {
      process.kill(pid, "SIGKILL");
    } catch {
      // Gone already.
    }
  }
}
