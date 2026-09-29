// Has git run the repository's hooks in .githooks (the pre-commit hook
// lints the site's staged files with ESLint, and formats them with
// Prettier). Run by `npm install` (`prepare`), so a clone that installs
// the site's dependencies gets them.
// Never fails the install: where there's no git, or no repository (a
// deploy's build, say), there's nothing to set up.
import {spawnSync} from "node:child_process";
import {existsSync} from "node:fs";
import {dirname, join} from "node:path";
import {fileURLToPath} from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
if (existsSync(join(root, ".githooks", "pre-commit"))) {
  const inRepo = spawnSync(
    "git",
    ["-C", root, "rev-parse", "--is-inside-work-tree"],
    {encoding: "utf8"},
  );
  if (inRepo.status === 0 && inRepo.stdout.trim() === "true") {
    spawnSync("git", ["-C", root, "config", "core.hooksPath", ".githooks"]);
  }
}
