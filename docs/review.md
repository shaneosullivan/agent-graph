# Adversarial review, September 2026

After phase 4, five fresh reviewers each attacked one part of the codebase: the local security surface, the sharing site, the reducer and adapters, the CLI across platforms, and the web viewer and images. Every finding below was reproduced or traced exactly. Where two reviewers found the same problem, it's listed once.

Each one is fixed on its own:
1. a test that fails without the fix;
2. the fix, with that test and the whole suite passing;
3. a fresh review of just the changed code;
4. its own commit, which names the finding (e.g. "R3").

Status is updated as each is done.

## High

### R1. `install` and `snapshot` write through symlinks a repo can plant
- **Where:** `src/store.rs` (`write_atomic`), `src/cli.rs` (`apply`, backups)
- **Problem:** Writes go to a predictable `<file>.tmp` with `fs::write`, which follows symlinks, then rename it into place; the backup `fs::copy` follows a symlink too. With `--scope project`/`local`, those paths are inside the repo, so a repo that ships `.claude/settings.tmp -> ~/.bashrc` gets `~/.bashrc` replaced with JSON it controls, which bash then runs.
- **Fix:** temporary files are always new (`create_new`, unique name) and renamed over the target, so a link there is replaced, not followed; backups are written the same way; and for project and local scope, a settings or command file that is, or goes through, a link in the project is refused before it's read. (A project whose `.claude` is deliberately a link now gets a clear error.)
- **Status:** fixed. Test: `install_never_writes_through_links_a_project_plants` (tests/cli.rs), plus `store` unit tests. Reviewed.

### R2. The `/agent-graph` skill pre-approves `agent-graph snapshot *`, whose `--out` overwrites any file
- **Where:** `src/slash.rs` (`allowed-tools`), `src/cli.rs` (`snapshot --out`)
- **Problem:** The wildcard lets a prompt-injected model run `agent-graph snapshot --json --out ~/.bashrc` with no permission prompt, writing task text it controls into any file.
- **Fix:** pre-approve only the exact forms the skill uses, without `--out`; make `--out` refuse to replace an existing file.
- **Status:** open

### R3. A parent cycle hides sessions and makes the reducer, `snapshot`, Save image, `tail` and the viewer loop forever
- **Where:** `src/reducer.rs` (explicit parents, `bind`, inferred agent parents, `cancel_unfinished_descendants`), `src/image.rs`, `src/live.rs`, `src/view/assets/app.js`
- **Problem:** An explicit parent (from `AGENT_GRAPH_PARENT`, e.g. `claude --resume A` run from A's child's shell), a spawn binding, or a crafted log can make a node its own ancestor. Nodes in the loop are never roots, so they vanish from every list, and anything that walks the tree from them never ends (memory grows until the process dies; a two-line pasted log hangs a site visitor's tab).
- **Fix:** never reparent a node under its own descendant; walk trees with a visited set everywhere.
- **Status:** open

### R4. A foreground spawn whose tool call fails stays "waiting" forever
- **Where:** `src/reducer.rs`
- **Problem:** A spawn's wait only closes on `spawn.returned`, which comes from `PostToolUse`, which Claude Code only fires on success. A failed or interrupted Agent/Bash call leaves the session "1 starting" forever, even after it goes idle or ends, and hides real staleness.
- **Fix:** close a node's open spawn waits when it goes idle or ends; ignore waits of finished nodes.
- **Status:** open

### R5. `install` overwrites a user's own command or skill file it can't read as UTF-8
- **Where:** `src/cli.rs` (slash-command plan)
- **Problem:** Any read error (not UTF-8, permission denied) is treated as "no file", so the user's own `agent-graph.toml` or `SKILL.md` is replaced without a backup, breaking the promise never to overwrite them.
- **Fix:** only "not found" means absent; look for our marker in the raw bytes; refuse to touch a file we can't read.
- **Status:** open

## Medium

### R6. `install`/`uninstall` make `settings.json` world-readable and replace a symlinked one with a copy
- **Where:** `src/store.rs` (`write_atomic`)
- **Problem:** The rewritten file gets default permissions (0644), exposing API keys kept in a 0600 file, and a dotfiles symlink is replaced by a regular file, so the real dotfile never gets the hooks.
- **Fix:** write to the symlink's target, keeping the original file's permissions.
- **Status:** open

### R7. The local viewer serves any local user
- **Where:** `src/view/mod.rs`, `src/view/http.rs`
- **Problem:** Loopback doesn't mean "this user": another account on the machine can read every log through the API and, forging `Origin`, open terminals on the user's desktop. There's also no limit on connections.
- **Fix:** a random token, printed in the viewer's URL and kept in a cookie, required for every request; cap concurrent connections.
- **Status:** open

### R8. Resume can run a `claude` planted in the session's folder
- **Where:** `src/view/open.rs`
- **Problem:** On Windows, `cmd` looks for commands in the current directory first, and the command runs in the session's (untrusted) folder, so a `claude.cmd` there runs instead. The same happens elsewhere when `PATH` has a relative entry.
- **Fix:** resolve the agent to an absolute path from `PATH` (ignoring relative entries) before launching.
- **Status:** open

### R9. `agent-graph run` un-ignores signals, so `nohup` and background jobs die
- **Where:** `src/run.rs`
- **Problem:** Handlers are installed for SIGINT, SIGQUIT, SIGHUP and SIGTERM even when they were inherited as ignored; the command then gets the default action instead of inheriting "ignored", and SIGHUP is forwarded to it.
- **Fix:** leave ignored signals ignored and don't forward them.
- **Status:** open

### R10. `--session current` falls back to the newest session anywhere, so `/agent-graph share` can publish another project
- **Where:** `src/cli.rs` (`pick_roots`), `src/remote.rs`
- **Problem:** When the current session isn't recorded, `current` quietly becomes the most recent session on the machine, which `watch-remote` then uploads.
- **Fix:** for sharing, `current` must match the session exactly or fail; always say which session is being shared.
- **Status:** open

### R11. On Windows, `run` can't start npm-installed agents (`codex`, `gemini`, `claude`)
- **Where:** `src/run.rs`
- **Problem:** `Command::new("codex")` only finds `codex.exe`; npm installs `codex.cmd`, so `run -- codex` fails with "program not found".
- **Fix:** on Windows, resolve the program through `PATH` and `PATHEXT`.
- **Status:** open

### R12. One invalid UTF-8 byte stops `tree`, `snapshot` and `watch-remote` for every session
- **Where:** `src/store.rs` (`load_events`)
- **Problem:** A write torn inside a multi-byte character makes `lines()` fail, and the whole load fails instead of skipping that line (the viewer already skips it).
- **Fix:** read lines as bytes and count the bad one as skipped.
- **Status:** open

### R13. `watch-remote --session X` doesn't share the sessions under X
- **Where:** `src/remote.rs`
- **Problem:** Only X's own file is uploaded, so sessions and `run` nodes linked under it (each in their own file) are missing from the shared view, and X looks like it's waiting on a child that never starts.
- **Fix:** share every file in X's tree, re-checked as new ones appear.
- **Status:** open

### R14. `install --scope project` writes this machine's binary path into a file meant to be committed
- **Where:** `src/cli.rs`, `src/install.rs`
- **Problem:** Teammates on other machines get a hook command that doesn't exist, so every hook fails for them.
- **Fix:** for project scope, use `agent-graph` from `PATH` unless `--command` says otherwise.
- **Status:** open

### R15. `watch-remote` silently drops events when a file can't be read
- **Where:** `src/remote.rs` (`Lines::poll`)
- **Problem:** Offsets of files read earlier in a poll are saved even when a later file fails, and the error is ignored, so those lines are never sent.
- **Fix:** keep going past a bad file, and report it.
- **Status:** open

### R16. A retried append can grow at the same offset, so viewers miss events and a late commit loses them
- **Where:** `src/remote.rs`, `site/lib/store.ts`, `site/app/api/logs/[id]/append/route.ts`
- **Problem:** A retry resends a bigger body at the same offset and the site overwrites the chunk; viewers past that chunk never see the extra lines, and a delayed original request can overwrite the retry, losing them for good.
- **Fix:** the client resends exactly the same bytes; the site never overwrites a chunk with different content.
- **Status:** open

### R17. One content request can return about 100 MB
- **Where:** `site/lib/store.ts` (`readChunks`)
- **Problem:** Reads are limited to 200 chunks, not bytes, so one unauthenticated request can pull ~100 MB through the function's memory.
- **Fix:** stop at a byte budget and let the client page.
- **Status:** open

### R18. Encryption at rest doesn't stop someone who can read the database
- **Where:** `site/lib/store.ts`
- **Problem:** Documents are keyed by the log's public id, which is also the link, so anyone with a database export can open every log through the site; the metadata (password hash) isn't authenticated either.
- **Fix:** key documents by an HMAC of the id, and authenticate the metadata.
- **Status:** open

### R19. Password unlock has no attempt limit
- **Where:** `site/app/api/logs/[id]/unlock/route.ts`
- **Problem:** Unlimited guesses, each costing a scrypt run on the server.
- **Fix:** limit attempts per log and per address, with a cooldown.
- **Status:** open

### R20. A slow timeline step can replace the graph after going Live, a cached step, or a session switch
- **Where:** `src/view/assets/app.js`
- **Problem:** The guard against out-of-order fetches is only bumped for uncached steps, so an older reply lands after the user moved on and shows the wrong graph under the right label.
- **Fix:** invalidate in-flight step fetches on every navigation.
- **Status:** open

### R21. A refresh in flight snaps the slider back to where it was
- **Where:** `src/view/assets/app.js` (`refresh`)
- **Problem:** `refresh` remembers the current step before its requests, so stepping meanwhile is undone, leaving the slider and the graph out of step.
- **Fix:** read the position after the requests return.
- **Status:** open

### R22. The reducer is quadratic in the number of nodes
- **Where:** `src/reducer.rs` (`bind_by_guess`, `close_waits_on`)
- **Problem:** Each agent start and finish scans every node; months of history take seconds per reduce, and every view reduces the whole log.
- **Fix:** look up a session's own nodes by range, and waits by target.
- **Status:** open

### R23. Every change, and every timeline step, reduces and sends the whole history
- **Where:** `src/timeline.rs`, `src/view/mod.rs`, `src/view/assets/app.js`
- **Problem:** Each request replays all events and returns every node of every session, with the events lock held; with a large history each step takes about half a second locally and freezes the site's page.
- **Fix:** send only the selected session's tree (plus a summary of the others), and compute outside the lock.
- **Status:** open

### R24. A shell-launched session can be paired with another program's request, for good
- **Where:** `src/reducer.rs` (`bind_session_by_guess`)
- **Problem:** With no request for its own program, a child takes the oldest open request of any program, including a background one from long ago, and takes its purpose and type.
- **Fix:** don't pair across known, different programs, and only pair background requests made shortly before.
- **Status:** open

### R25. The shell parser sees launches that aren't
- **Where:** `src/adapter/shell.rs`
- **Problem:** Heredoc bodies (every commit message Claude writes) are parsed as commands, names are matched case-insensitively ("Claude & Codex …"), `command -v claude` counts, and so do other tools called `goose` or `copilot`.
- **Fix:** skip heredoc bodies, match names exactly, treat `command -v` as a lookup, and skip those tools' non-agent subcommands.
- **Status:** open

### R26. The shell parser misses launches after `"$(…)"` and line continuations
- **Where:** `src/adapter/shell.rs`
- **Problem:** A substitution inside double quotes flips the quote state for the rest of the command, and a backslash-newline becomes a word.
- **Fix:** track quotes per substitution level; drop backslash-newlines.
- **Status:** open

### R27. A long TodoWrite list is dropped entirely
- **Where:** `src/emit.rs` (`to_line`)
- **Problem:** A list over 4 KB (about 18 ordinary items) becomes an `unknown` event, so the task list and headline go stale.
- **Fix:** shrink it (shorter text, then fewer items, marked) so it keeps its type.
- **Status:** open

### R28. A status sorted after `session.ended` brings the session back to life
- **Where:** `src/reducer.rs`
- **Problem:** A headless session's Stop and SessionEnd can land in the same millisecond in either order; if Stop sorts last, the session shows as idle (alive) forever.
- **Fix:** only a new `session.started` revives an ended session.
- **Status:** open

## Low

### R29. A `--command` without the usual form duplicates hooks and can't be uninstalled
- **Where:** `src/install.rs`
- **Status:** open

### R30. `uninstall --scope local` deletes the project's skill
- **Where:** `src/slash.rs`, `src/cli.rs`
- **Status:** open

### R31. `snapshot --session X --out x.json` writes every session
- **Where:** `src/cli.rs`
- **Status:** open

### R32. Two snapshots in the same second overwrite each other
- **Where:** `src/cli.rs`
- **Status:** open

### R33. U+FFFE or U+FFFF in any text breaks every image of that session
- **Where:** `src/image.rs` (`escape`)
- **Status:** open

### R34. The per-step graph cache grows without limit
- **Where:** `src/view/assets/app.js`
- **Status:** open

### R35. Re-rendering scrolls the tree back to the ringed card
- **Where:** `src/view/assets/app.js`
- **Status:** open

### R36. A malformed URL hash blanks the viewer
- **Where:** `src/view/assets/app.js` (`rootFromHash`)
- **Status:** open

### R37. Long unbroken text overflows cards and panels
- **Where:** `src/view/assets/app.css`
- **Status:** open

### R38. Live updates drop keyboard focus and text selection
- **Where:** `src/view/assets/app.js`
- **Status:** open

### R39. The timeline's hover tip shows labels without cleaning them
- **Where:** `src/view/assets/app.js` (`showTip`, `aria-valuetext`)
- **Status:** open

### R40. Snapshot layout: the stale flag overlaps, deep trees get negative widths, agents are uncapped
- **Where:** `src/image.rs`
- **Status:** open

### R41. Session requests only pair at a child's first start
- **Where:** `src/reducer.rs`
- **Problem:** A child resumed from the parent's shell, or one whose start sorts just before the request, is never paired.
- **Status:** open

### R42. `claude … & claude … & wait` counts as a background launch
- **Where:** `src/adapter/shell.rs`
- **Status:** open

### R43. A node id with thousands of `/` overflows the stack
- **Where:** `src/reducer.rs` (`ensure`)
- **Status:** open

### R44. The log size cap doesn't bound storage
- **Where:** `site/app/api/logs/[id]/append/route.ts`
- **Status:** open

### R45. Create and unlock buffer the whole body when there's no Content-Length
- **Where:** `site/app/api/logs/**`
- **Status:** open

### R46. Passwords of 1025–1050 bytes are accepted but can never unlock
- **Where:** `site/lib/crypto.ts`, unlock route
- **Status:** open

### R47. The CLI's refusal to send a password over plain HTTP can be bypassed
- **Where:** `src/remote.rs`
- **Status:** open

### R48. `tail` leaves the terminal broken when killed
- **Where:** `src/live.rs`
- **Status:** open

### R49. Every install or uninstall overwrites the backup, losing the original
- **Where:** `src/cli.rs`
- **Status:** open

### R50. On Windows, `run` turns exit codes above 255 into 1
- **Where:** `src/run.rs`
- **Status:** open

### R51. `uninstall` leaves behind the empty folders `install` created
- **Where:** `src/cli.rs`
- **Status:** open
