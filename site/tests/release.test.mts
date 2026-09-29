// Unit tests for lib/release.ts:  npm run test:unit

import assert from "node:assert/strict";
import {execFileSync, spawnSync} from "node:child_process";
import {createHash} from "node:crypto";
import {mkdtempSync, readFileSync, writeFileSync, chmodSync} from "node:fs";
import {tmpdir} from "node:os";
import {join} from "node:path";
import {pathToFileURL} from "node:url";
import {test} from "node:test";

const {installScript, latestRelease} = await import("../lib/release.ts");

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
