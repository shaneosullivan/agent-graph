// Unit tests for lib/release.ts:  npm run test:unit

import assert from "node:assert/strict";
import {execFile, execFileSync, spawnSync} from "node:child_process";
import {createHash} from "node:crypto";
import {
  chmodSync,
  existsSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import {createServer} from "node:http";
import type {AddressInfo} from "node:net";
import {promisify} from "node:util";
import {tmpdir} from "node:os";
import {join} from "node:path";
import {pathToFileURL} from "node:url";
import {test} from "node:test";

const {installScript, latestRelease, powerShellScript, hasWindows} =
  await import("../lib/release.ts");

const TARGETS = [
  "aarch64-apple-darwin",
  "x86_64-apple-darwin",
  "aarch64-unknown-linux-musl",
  "x86_64-unknown-linux-musl",
] as const;

test("no release before the first one", () => {
  assert.equal(
    latestRelease({version: null, commit: null, date: null, files: {}}),
    null,
  );
  const script = installScript(null);
  const run = spawnSync("sh", ["-c", script], {encoding: "utf8"});
  assert.notEqual(run.status, 0);
  assert.match(run.stderr, /hasn't been released yet/);
});

test("release.json is a release, or the one before the first", () => {
  const json = JSON.parse(
    readFileSync(new URL("../release.json", import.meta.url), "utf8"),
  );
  const release = latestRelease(json);
  if (release) {
    assert.match(release.version, /^\d+\.\d+\.\d+/);
    for (const target of TARGETS) {
      assert.match(release.files[target]!.url, /^https:\/\//);
      assert.match(release.files[target]!.sha256, /^[0-9a-f]{64}$/);
    }
  } else {
    assert.equal(json.version, null);
  }
});

// The install script is for macOS and Linux (it says so, and stops, on
// Windows), and Windows' tar takes C: for a host.
const unix = {
  skip:
    process.platform === "win32" && "the install script is for macOS and Linux",
};

/** A release of a stand-in program, archived as release.sh archives one. */
function fakeRelease(dir: string, sha256?: string) {
  const program = join(dir, "agent-graph");
  writeFileSync(program, '#!/bin/sh\necho "agent-graph 9.9.9-beta.1"\n');
  chmodSync(program, 0o755);
  const archive = join(dir, "agent-graph.tar.gz");
  execFileSync("tar", ["-czf", archive, "-C", dir, "agent-graph"]);
  const sha =
    sha256 ?? createHash("sha256").update(readFileSync(archive)).digest("hex");
  const file = {url: pathToFileURL(archive).href, sha256: sha};
  return {
    version: "9.9.9-beta.1",
    commit: "0".repeat(40),
    date: "2026-09-29",
    files: Object.fromEntries(TARGETS.map(t => [t, file])),
  };
}

test("the install script installs the release for this machine", unix, () => {
  const dir = mkdtempSync(join(tmpdir(), "ag-release-"));
  const into = join(dir, "bin");
  const run = spawnSync("sh", ["-c", installScript(fakeRelease(dir))], {
    encoding: "utf8",
    env: {...process.env, AGENT_GRAPH_INSTALL_DIR: into},
  });
  assert.equal(run.status, 0, run.stderr);
  assert.match(
    run.stdout,
    /Installed .*agent-graph \(agent-graph 9\.9\.9-beta\.1\)/,
  );
  assert.equal(
    execFileSync(join(into, "agent-graph"), {encoding: "utf8"}).trim(),
    "agent-graph 9.9.9-beta.1",
  );
});

test(
  "the install script refuses a download that isn't the release's",
  unix,
  () => {
    const dir = mkdtempSync(join(tmpdir(), "ag-release-"));
    const into = join(dir, "bin");
    const run = spawnSync(
      "sh",
      ["-c", installScript(fakeRelease(dir, "0".repeat(64)))],
      {
        encoding: "utf8",
        env: {...process.env, AGENT_GRAPH_INSTALL_DIR: into},
      },
    );
    assert.notEqual(run.status, 0);
    assert.match(run.stderr, /SHA-256 isn't the release's: not installed/);
    assert.equal(
      spawnSync("test", ["-e", join(into, "agent-graph")]).status,
      1,
    );
  },
);

test("a quote in a URL can't break out of the script", () => {
  const script = installScript({
    version: "1.0.0",
    commit: "0".repeat(40),
    date: "2026-09-29",
    files: {
      "aarch64-apple-darwin": {
        url: "https://x/'; touch /tmp/pwned; '",
        sha256: "a",
      },
    },
  });
  assert.ok(
    script.includes(`url='https://x/'\\''; touch /tmp/pwned; '\\'''`),
    script,
  );
});

// --- Windows: /install.ps1 ---------------------------------------------------

const WINDOWS = ["aarch64-pc-windows-msvc", "x86_64-pc-windows-msvc"] as const;

const someRelease = (files: Record<string, {url: string; sha256: string}>) => ({
  version: "9.9.9",
  commit: "0".repeat(40),
  date: "2026-10-02",
  files,
});

test("without a release for Windows, the PowerShell script says so", () => {
  assert.match(
    powerShellScript(null),
    /^Write-Error 'agent-graph hasn''t been released yet/,
  );
  const unixOnly = someRelease({
    "aarch64-apple-darwin": {url: "https://x/a.tar.gz", sha256: "a"},
  });
  assert.equal(hasWindows(unixOnly), false);
  assert.match(
    powerShellScript(unixOnly),
    /^Write-Error 'There''s no agent-graph 9\.9\.9 for Windows yet/,
  );
});

test("the PowerShell script has the Windows builds, and only them; install.sh has the others", () => {
  const release = someRelease({
    "aarch64-apple-darwin": {url: "https://x/mac.tar.gz", sha256: "m"},
    "x86_64-pc-windows-msvc": {url: "https://x/win-x64.zip", sha256: "w1"},
    "aarch64-pc-windows-msvc": {url: "https://x/win-arm.zip", sha256: "w2"},
  });
  assert.equal(hasWindows(release), true);
  const ps = powerShellScript(release);
  assert.ok(
    ps.includes("agent-graph 9.9.9"),
    "release.sh looks for its version",
  );
  assert.ok(
    ps.includes(
      `'x86_64-pc-windows-msvc' = @{ Url = 'https://x/win-x64.zip'; Sha = 'w1' }`,
    ),
    ps,
  );
  assert.ok(
    ps.includes(
      `'aarch64-pc-windows-msvc' = @{ Url = 'https://x/win-arm.zip'; Sha = 'w2' }`,
    ),
  );
  assert.ok(!ps.includes("mac.tar.gz"));
  const sh = installScript(release);
  assert.ok(sh.includes("mac.tar.gz"));
  assert.ok(!sh.includes("windows-msvc"), "install.sh offers no Windows build");
  assert.ok(sh.includes("install.ps1 | iex"), "it says what to run on Windows");
});

test("a quote in a URL can't break out of the PowerShell script", () => {
  const ps = powerShellScript(
    someRelease({
      "x86_64-pc-windows-msvc": {
        url: "https://x/'; Remove-Item C:\; '",
        sha256: "a",
      },
    }),
  );
  assert.ok(ps.includes("Url = 'https://x/''; Remove-Item C:\; '''"), ps);
});

// Run for real only on Windows (CI's runner), in Windows PowerShell.
const windowsOnly = {
  skip: process.platform !== "win32" && "the PowerShell script is for Windows",
};

test(
  "the PowerShell script installs the release for this machine",
  windowsOnly,
  async () => {
    const run = promisify(execFile);
    const dir = mkdtempSync(join(tmpdir(), "ag-ps-"));
    // A stand-in program, zipped as release.sh zips one.
    writeFileSync(join(dir, "agent-graph.exe"), "stand-in 1");
    writeFileSync(join(dir, "LICENSE"), "MIT");
    const zip = join(dir, "agent-graph.zip");
    await run("powershell", [
      "-NoProfile",
      "-Command",
      `Compress-Archive -Path '${join(dir, "agent-graph.exe")}','${join(dir, "LICENSE")}' -DestinationPath '${zip}'`,
    ]);
    const bytes = readFileSync(zip);
    const sha256 = createHash("sha256").update(bytes).digest("hex");
    const server = createServer((_req, res) => res.end(bytes));
    await new Promise<void>(resolve => server.listen(0, "127.0.0.1", resolve));
    const url = `http://127.0.0.1:${(server.address() as AddressInfo).port}/agent-graph.zip`;
    const into = join(dir, "bin");
    // The script for a release whose Windows builds are the stand-in, with
    // `sha` as their SHA-256, run as a file installing into `to`.
    const install = (sha: string, to: string) => {
      const script = join(dir, `install-${sha.slice(0, 8)}.ps1`);
      writeFileSync(
        script,
        powerShellScript(
          someRelease(
            Object.fromEntries(WINDOWS.map(t => [t, {url, sha256: sha}])),
          ),
        ),
      );
      return run(
        "powershell",
        ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", script],
        {env: {...process.env, AGENT_GRAPH_INSTALL_DIR: to}},
      );
    };
    try {
      const first = await install(sha256, into);
      assert.match(
        first.stdout,
        /Installed agent-graph 9\.9\.9 as .*agent-graph\.exe/,
      );
      // Not the default folder: it says to add it, and leaves PATH alone.
      assert.match(first.stdout, /Add .* to your PATH/);
      assert.equal(
        readFileSync(join(into, "agent-graph.exe"), "utf8"),
        "stand-in 1",
      );

      // Again: the one there's moved aside (it may be running), not refused.
      await install(sha256, into);
      assert.equal(
        readFileSync(join(into, "agent-graph.exe"), "utf8"),
        "stand-in 1",
      );
      assert.equal(
        readdirSync(into).filter(f => f.startsWith("agent-graph.exe.old-"))
          .length,
        1,
      );

      // A download that isn't the release's isn't installed.
      const wrong = join(dir, "wrong");
      await assert.rejects(
        install("0".repeat(64), wrong),
        /SHA-256 isn't the release's/,
      );
      assert.equal(existsSync(join(wrong, "agent-graph.exe")), false);
    } finally {
      server.close();
    }
  },
);
