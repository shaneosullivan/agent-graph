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
- **Fix:** the skill pre-approves only the exact commands its instructions run (`tree`, `tree --all`, `snapshot --session current`, `snapshot --all`); `--out` only writes .png, .svg or .json, and never replaces an existing file (or a link) unless given the new `--force`. A skill installed before this keeps the old rules until `agent-graph install` is run again.
- **Status:** fixed. Tests: `claude_pre_approves_only_local_commands` (src/slash.rs), `snapshot_out_never_replaces_a_file_unless_forced` (tests/cli.rs). Reviewed.

### R3. A parent cycle hides sessions and makes the reducer, `snapshot`, Save image, `tail` and the viewer loop forever
- **Where:** `src/reducer.rs` (explicit parents, `bind`, inferred agent parents, `cancel_unfinished_descendants`), `src/image.rs`, `src/live.rs`, `src/view/assets/app.js`
- **Problem:** An explicit parent (from `AGENT_GRAPH_PARENT`, e.g. `claude --resume A` run from A's child's shell), a spawn binding, or a crafted log can make a node its own ancestor. Nodes in the loop are never roots, so they vanish from every list, and anything that walks the tree from them never ends (memory grows until the process dies; a two-line pasted log hangs a site visitor's tab).
- **Fix:** `reparent`, the one place parents change, refuses a parent that's the node itself or below it (a resumed session stays on top, and its link isn't recorded); `bind` won't bind a child that's above its requester; and every tree walker (image, text tree, `tail`, the viewer, cancelling descendants) skips nodes it has already visited. Added a jsdom harness for testing the viewer (site/tests/viewer-harness.mjs).
- **Status:** fixed. Tests: four in tests/sessions.rs (`a_session_resumed_from_its_childs_shell_stays_on_top`, `a_session_cant_be_under_its_own_agent`, `a_crafted_parent_loop_doesnt_hang_the_reducer`, `drawing_a_graph_with_a_loop_finishes`) and "R3: …" in site/tests/viewer.test.mjs. Reviewed.

### R4. A foreground spawn whose tool call fails stays "waiting" forever
- **Where:** `src/reducer.rs`
- **Problem:** A spawn's wait only closes on `spawn.returned`, which comes from `PostToolUse`, which Claude Code only fires on success. A failed or interrupted Agent/Bash call leaves the session "1 starting" forever, even after it goes idle or ends, and hides real staleness.
- **Fix:** when a node goes idle or ends (including agents canceled with their session), every foreground call it made is over: its waits close and the requests can't be paired with a later child. A finished node is never blocked, and waiting on a session no longer counts what its finished agents were waiting for.
- **Status:** fixed. Tests: `a_spawn_that_never_returns_stops_blocking_when_the_turn_ends`, `an_ended_session_is_never_blocked`, `a_dead_request_isnt_paired_with_a_later_child`, `a_canceled_agents_request_isnt_paired_after_a_resume`, `a_finished_agents_old_wait_isnt_counted` (tests/sessions.rs). Reviewed.

### R5. `install` overwrites a user's own command or skill file it can't read as UTF-8
- **Where:** `src/cli.rs` (slash-command plan)
- **Problem:** Any read error (not UTF-8, permission denied) is treated as "no file", so the user's own `agent-graph.toml` or `SKILL.md` is replaced without a backup, breaking the promise never to overwrite them.
- **Fix:** the file is read as bytes and our marker looked for in them; only a missing file counts as absent, and one that can't be read is left alone, with a note saying so.
- **Status:** fixed. Test: `install_leaves_a_command_file_it_cant_read_alone` (tests/cli.rs). Reviewed.

## Medium

### R6. `install`/`uninstall` make `settings.json` world-readable and replace a symlinked one with a copy
- **Where:** `src/store.rs` (`write_atomic`)
- **Problem:** The rewritten file gets default permissions (0644), exposing API keys kept in a 0600 file, and a dotfiles symlink is replaced by a regular file, so the real dotfile never gets the hooks.
- **Fix:** a rewritten regular file keeps its Unix permissions, and anything new (including a file replacing a link) is created readable only by the user; the backup gets the original's permissions. A settings file of your own that's a link is updated where it points, with the backup kept beside the link (a project's links are still refused, per R1). On Windows no attributes are copied, so a read-only file can't block the next run.
- **Status:** fixed. Tests: `install_keeps_settings_private_and_follows_the_users_own_link` (tests/cli.rs), `a_replaced_link_doesnt_take_its_targets_permissions` and (Windows) `a_read_only_original_doesnt_stop_the_next_write` (src/store.rs). Reviewed.

### R7. The local viewer serves any local user
- **Where:** `src/view/mod.rs`, `src/view/http.rs`
- **Problem:** Loopback doesn't mean "this user": another account on the machine can read every log through the API and, forging `Origin`, open terminals on the user's desktop. There's also no limit on connections.
- **Fix:** each run makes a random 160-bit key and prints its link as `http://127.0.0.1:<port>/?key=…`. The page (public, built in) keeps the key in its own origin's storage and sends it with every API request as an `X-Agent-Graph-Key` header, which another website can't send; only the event stream takes it as `?key=`, and Save image fetches with the header so the key isn't recorded on the file. Not a cookie: browsers send cookies to every localhost port. The viewer listens on `[::1]` too and refuses to start if another program holds it (browsers try `localhost` as `[::1]` first). Connections are capped at 256, a request's head must arrive within 5 s, and a panicking handler gives its slot back. (A local user without the key can still lock the viewer out by flooding it, but can't read anything.)
- **Status:** fixed. Tests: `nothing_private_is_served_without_the_viewers_key`, `connections_past_the_limit_are_closed`, `the_viewer_wont_share_its_port_on_ipv6_localhost` (tests/view.rs), `a_trickled_request_is_cut_off`, `a_panicking_handler_gives_its_slot_back` (src/view), and "R7: …" in site/tests/viewer.test.mjs. Reviewed (three rounds: the first design used a cookie, which leaks to other localhost servers; the second missed IPv6 `localhost`).

### R8. Resume can run a `claude` planted in the session's folder
- **Where:** `src/view/open.rs`
- **Problem:** On Windows, `cmd` looks for commands in the current directory first, and the command runs in the session's (untrusted) folder, so a `claude.cmd` there runs instead. The same happens elsewhere when `PATH` has a relative entry.
- **Fix:** the agent (and on Linux the terminal) is found on `PATH` by full path, skipping relative entries (and trying `PATHEXT` on Windows), and run by that path; the agent gets a `PATH` without relative entries (the macOS and Linux scripts rebuild it themselves, never empty); Windows launches `%ComSpec%` with `NoDefaultCurrentDirectoryInExePath`, and refuses a path with characters `cmd` can't be trusted to pass along. The "run it yourself" command names the agent by full path too.
- **Status:** fixed. Tests: `the_agent_is_found_by_full_path_and_never_in_the_folder`, `relative_path_entries_are_dropped_for_the_agent`, `paths_cmd_would_misread_are_refused`, `the_command_to_type_yourself` (src/view/open.rs). Reviewed (three rounds).

### R9. `agent-graph run` un-ignores signals, so `nohup` and background jobs die
- **Where:** `src/run.rs`
- **Problem:** Handlers are installed for SIGINT, SIGQUIT, SIGHUP and SIGTERM even when they were inherited as ignored; the command then gets the default action instead of inheriting "ignored", and SIGHUP is forwarded to it.
- **Fix:** each signal's current disposition is read first; one inherited as ignored is left alone (no handler, nothing forwarded), so the command inherits "ignored" too.
- **Status:** fixed. Test: `run_keeps_signals_that_were_ignored_ignored` (tests/cli.rs). Reviewed.

### R10. `--session current` falls back to the newest session anywhere, so `/agent-graph share` can publish another project
- **Where:** `src/cli.rs` (`pick_roots`), `src/remote.rs`
- **Problem:** When the current session isn't recorded, `current` quietly becomes the most recent session on the machine, which `watch-remote` then uploads.
- **Fix:** for sharing, `current` means only the Claude Code session this runs in (`CLAUDE_CODE_SESSION_ID`, recorded); otherwise nothing is shared and the user is asked to choose. The /agent-graph skill tells the model to ask the user, never to pick a session itself. A `--session` prefix matches sessions only (not an agent in another project's session), and the shared session is named (cleaned) before it's sent. The local views (`tree`, `tail`, `snapshot`) keep their friendlier fallback.
- **Status:** fixed. Tests: `sharing_the_current_session_never_falls_back_to_another`, `a_session_prefix_never_matches_another_sessions_agent`, `the_shared_sessions_description_is_cleaned` (tests/remote.rs), `claude_pre_approves_only_local_commands` (src/slash.rs). Reviewed.

### R11. On Windows, `run` can't start npm-installed agents (`codex`, `gemini`, `claude`)
- **Where:** `src/run.rs`
- **Problem:** `Command::new("codex")` only finds `codex.exe`; npm installs `codex.cmd`, so `run -- codex` fails with "program not found".
- **Fix:** on Windows, a bare program name is looked up through `PATH` and `PATHEXT` (as `cmd` would, using absolute entries) and its full path started; elsewhere nothing changes. The resolver is shared with Resume (R8), now in `paths.rs`.
- **Status:** fixed. Test: `run_starts_a_cmd_file_by_its_bare_name` (tests/cli.rs, runs on Windows CI). Reviewed.

### R12. One invalid UTF-8 byte stops `tree`, `snapshot` and `watch-remote` for every session
- **Where:** `src/store.rs` (`load_events`)
- **Problem:** A write torn inside a multi-byte character makes `lines()` fail, and the whole load fails instead of skipping that line (the viewer already skips it).
- **Fix:** lines are read as bytes and parsed with `from_slice`, so one that isn't UTF-8 is a skipped line like any other that doesn't parse (as the viewer already did).
- **Status:** fixed. Test: `a_line_that_isnt_utf8_is_skipped_not_fatal` (src/store.rs). Reviewed.

### R13. `watch-remote --session X` doesn't share the sessions under X
- **Where:** `src/remote.rs`
- **Problem:** Only X's own file is uploaded, so sessions and `run` nodes linked under it (each in their own file) are missing from the shared view, and X looks like it's waiting on a child that never starts.
- **Fix:** every file in X's tree is shared (X's, and those of sessions and `run`s linked under it). Each poll reads the tree's new lines first, then works the tree out again once (only if a file outside it changed, at most every 3 s, or a member restarted and so may have left), reads any files that joined, and sends only what's still in the tree. A session that leaves stops being shared; while the log can't be read, anything that may have left is held back (with a warning), and nothing is lost.
- **Status:** fixed. Tests: `sharing_a_session_includes_the_sessions_under_it`, `a_session_that_leaves_the_shared_tree_stops_being_shared` (tests/remote.rs), and `a_tree_is_worked_out_once_per_poll`, `a_restarted_member_is_held_back_until_the_tree_can_be_checked`, `a_failed_recheck_tries_again`, `a_session_id_without_a_provider_is_refused` (src/remote.rs). Reviewed (four rounds).

### R14. `install --scope project` writes this machine's binary path into a file meant to be committed
- **Where:** `src/cli.rs`, `src/install.rs`
- **Problem:** Teammates on other machines get a hook command that doesn't exist, so every hook fails for them.
- **Fix:** project-scope hooks run `agent-graph` from `PATH` (unless `--command` says otherwise), and the install says everyone needs it installed, warning if it isn't on your `PATH` or is a different copy. User and local scope keep this copy's full path, since hooks run with Claude Code's own `PATH`. Someone with both user and project hooks has each event recorded twice, so the reducer now counts a repeated message once (spawns, waits and tasks already were).
- **Status:** fixed. Tests: `project_hooks_run_agent_graph_from_path`, `user_hooks_keep_the_full_path_even_when_path_has_this_copy`, `project_install_warns_when_agent_graph_isnt_on_path` (tests/cli.rs), `a_message_recorded_twice_counts_once` (tests/sessions.rs). Reviewed (three rounds).

### R15. `watch-remote` silently drops events when a file can't be read
- **Where:** `src/remote.rs` (`Lines::poll`)
- **Problem:** Offsets of files read earlier in a poll are saved even when a later file fails, and the error is ignored, so those lines are never sent.
- **Fix:** keep going past a bad file, and report it.
- **Status:** fixed. A file that can't be read is skipped, said once, and retried; its offset doesn't move, so nothing is lost. Sharing one session's tree: a member that can't be read holds back the rest until a recheck drops it (with the sessions under it, which may have moved along), and rejoins once it can be read; `--session` takes only a whole id while any file can't be read; `load_events` leaves out, and names, a file it can't read. Tests: `a_file_that_cant_be_read_is_left_out` (src/store.rs), `a_file_that_cant_be_read_doesnt_stop_the_rest`, `an_unreadable_file_elsewhere_doesnt_block_the_tree`, `a_deleted_file_is_forgotten`, `a_restarted_member_is_held_back_until_the_tree_can_be_checked`, `an_unreadable_member_drops_out_and_the_rest_carry_on`, `a_member_that_can_be_read_again_rejoins`, `a_session_under_an_unreadable_one_drops_out_too`, `a_file_the_recheck_can_read_again_is_forgotten`, `a_prefix_is_refused_while_a_file_cant_be_read`, `a_failed_recheck_tries_again` (src/remote.rs). Reviewed (three rounds).

### R16. A retried append can grow at the same offset, so viewers miss events and a late commit loses them
- **Where:** `src/remote.rs`, `site/lib/store.ts`, `site/app/api/logs/[id]/append/route.ts`
- **Problem:** A retry resends a bigger body at the same offset and the site overwrites the chunk; viewers past that chunk never see the extra lines, and a delayed original request can overwrite the retry, losing them for good.
- **Fix:** the client resends exactly the same bytes; the site never overwrites a chunk with different content.
- **Status:** fixed. A failed chunk is retried with exactly the same bytes, whatever has arrived since. The site creates each chunk (never sets it): the same bytes again are accepted, and other bytes at an offset it already has, or a stored chunk it can't decrypt, are refused with `409`, which the client treats as fatal. Tests: `a_retried_append_sends_the_same_bytes` (tests/remote.rs); "retrying a chunk doesn't duplicate it", "a chunk is never replaced with different bytes", "the same chunk sent several times at once is stored once", "a stored chunk that can't be decrypted isn't replaced, and says so" (site/tests/api.test.mjs). Reviewed.

### R17. One content request can return about 100 MB
- **Where:** `site/lib/store.ts` (`readChunks`)
- **Problem:** Reads are limited to 200 chunks, not bytes, so one unauthenticated request can pull ~100 MB through the function's memory.
- **Fix:** stop at a byte budget and let the client page.
- **Status:** fixed. A read fetches chunks `CHUNKS_PER_QUERY` (16) at a time, so a query holds at most 16 × 512 KB (8 MiB) at once, besides the up to 2.5 MiB being returned, and stops once it has `BYTES_PER_READ` (2 MiB, plus the chunk that crosses it), saying there's more, having fetched at most 15 chunks it didn't use; this holds however the log's chunks were written (overlapping offsets too). The viewer already pages on `X-More`. (Streaming the query instead doesn't work: leaving a Firestore stream doesn't cancel it.) Tests: "a read stops at a byte budget, and paging gets the rest" (site/tests/api.test.mjs), "a read fetches chunks a few at a time", "a read of many small chunks carries on across queries" (site/tests/store.test.mjs, against the emulator). Reviewed (three rounds).

### R18. Encryption at rest doesn't stop someone who can read the database
- **Where:** `site/lib/store.ts`
- **Problem:** Documents are keyed by the log's public id, which is also the link, so anyone with a database export can open every log through the site; the metadata (password hash) isn't authenticated either.
- **Fix:** key documents by an HMAC of the id, and authenticate the metadata.
- **Status:** fixed. Logs are stored under `storageId(id)`, an HMAC of the id with a key derived from `AGENT_GRAPH_ENCRYPTION_KEY` (so rotating `AGENT_GRAPH_SECRET` still doesn't touch stored data), and each log's metadata carries `mac`, an HMAC of its id, source and password hash, checked on every read: metadata that doesn't match is treated as missing. The site's own log messages name a log by its storage id, never its id (the platform's request logs still have `/l/{id}` in their paths, and the migration names old logs by their ids, which are their documents' names). `npm run migrate:storage-ids` copies logs stored the old way (before and after deploying), writing each one's metadata last so it shows only once complete, and `-- --delete-old` then deletes the old copies, each log's only once they're all checked; it refuses to run without `AGENT_GRAPH_ENCRYPTION_KEY` (outside the emulator), checks the key decrypts the old logs before writing anything, and reports and skips a log it can't move (any failed write included); a log without chunks is deleted only once another log's chunks, or its copy's, prove the key. Tests: "a log's storage id is keyed", "a metadata tag covers the id, the source and the password", "storage ids and tags are keyed apart" (site/tests/encryption.test.mts); "a log isn't stored under its id, and its chunks are ciphertext", "metadata changed in the database is refused", "the migration copies logs, then deletes the old copies", "the migration changes nothing with the wrong key, or none", "a log the migration can't move is left as it was, and the rest are moved", "a log without chunks is deleted only once the key is proven", "a stored chunk that can't be decrypted isn't replaced" (moved here from the API tests, since the chunk's path now needs the key) (site/tests/store.test.mjs); "logs are encrypted in Firestore, and not stored under their ids" (site/tests/api.test.mjs). Reviewed (three rounds).

### R19. Password unlock has no attempt limit
- **Where:** `site/app/api/logs/[id]/unlock/route.ts`
- **Problem:** Unlimited guesses, each costing a scrypt run on the server.
- **Fix:** limit attempts per log and per address, with a cooldown.
- **Status:** fixed. Each guess is counted before the password is checked, in a Firestore transaction against three buckets per 15-minute window: the log and address together (`UNLOCKS_PER_LOG_AND_ADDRESS`, 5), the log (`UNLOCKS_PER_LOG`, 20) and the address (`UNLOCKS_PER_ADDRESS`, 30); a right guess is given back, so only wrong ones use up the limits, and one address can only hold itself back. Past a limit, unlocking answers `429` with `Retry-After` (the page shows its message) and runs no scrypt; refused guesses aren't counted, a full bucket is refused by a plain read (no transaction to contend over), and too many guesses at once to count get a short `429`, not an error. The route's logic is in `lib/unlock.ts`, with what it uses passed in. Bucket documents are keyed (the log's storage id, HMACs of the address) and carry `expireAt` for a TTL policy. The address is `X-Real-IP` (which Vercel sets), IPv6 by its /64; without one, guesses are limited per log only. Tests: "a refused guess isn't checked", "a guess is counted, then checked, and a right one given back", "a password that can't be right isn't counted or checked", "guesses are counted by the platform's address, by block", "an IPv6 address is counted by its /64" (site/tests/unlock.test.mts); "unlocking a log is limited, from any number of addresses", "one address's guesses at a log hold back only that address", "unlocking is limited per address, across logs", "right passwords don't use up the limits" (site/tests/api.test.mjs); "password guesses are counted once each, and the count ends with its window", "a right guess is given back", "guesses that can't be counted just now wait a moment" (site/tests/store.test.mjs). Reviewed (three rounds). A full bucket counted in during the last 10 s (right guesses among those are about to be given back), or too many guesses at once to count, is a 5 s wait ("a few seconds"), not the window's end. Right guesses aren't limited, only how many are checked at once: see R52.

### R20. A slow timeline step can replace the graph after going Live, a cached step, or a session switch
- **Where:** `src/view/assets/app.js`
- **Problem:** The guard against out-of-order fetches is only bumped for uncached steps, so an older reply lands after the user moved on and shows the wrong graph under the right label.
- **Fix:** invalidate in-flight step fetches on every navigation.
- **Status:** fixed. `forgetLoads()` (clear the pending timer, bump `S.seq`) runs whenever the view moves on: every `goTo` (live and cached steps included), `selectRoot`, and a refresh that finds the session gone (before it waits for the next one's timeline); `selectRoot` and that refresh also show the live graph straight away and drop the old session's timeline, so it can't be stepped through meanwhile. A refresh that stays on a past step keeps its load. Each refresh starts a new cache map, so a step that was loading lands in the old one, and a step left behind that fails shows no error. Tests: "R20: a step's graph arriving late never replaces the graph shown since" (site/tests/viewer.test.mjs): going live, a cached step, the session going, and choosing another session, each while a step loads (the step arriving before the new timeline); stepping while the new session's timeline comes; choosing a session from a past step; a refresh that stays on the loading step; a left step's failure (silent) and the viewed step's (shown); and a step loaded before a refresh isn't served from the new cache. Reviewed (three rounds).

### R21. A refresh in flight snaps the slider back to where it was
- **Where:** `src/view/assets/app.js` (`refresh`)
- **Problem:** `refresh` remembers the current step before its requests, so stepping meanwhile is undone, leaving the slider and the graph out of step.
- **Fix:** read the position after the requests return.
- **Status:** fixed. `refresh` reads which stop is being viewed only once its requests are back, so a move made meanwhile stands (mapped by id onto the new timeline). When that stop has left the timeline, it goes back to the nearest earlier one still there and loads it (through `goTo`, which also forgets the old one's load), rather than on by index, which could land on the newest and go live; an empty timeline goes live. A refresh overtaken by a change of session shows nothing of the old session's timeline, and says nothing if it fails; the one that follows shows the new session, which `selectRoot` (and a refresh that finds the session gone) renders straight away, with the timeline's controls reset; `selectRoot` ignores a session the live graph no longer has. Tests: "R21: stepping while a refresh is in flight isn't undone", "R21: when the step being viewed leaves the timeline, the nearest one before it is shown", "R21: a refresh overtaken by a change of session shows nothing of the old one's timeline", "R21: choosing a session that has just gone is ignored, and the page recovers", "R21: a session's controls start afresh when it's chosen" (site/tests/viewer.test.mjs), and the R20 test's session-gone step. Reviewed (three rounds).

### R22. The reducer is quadratic in the number of nodes
- **Where:** `src/reducer.rs` (`bind_by_guess`, `close_waits_on`)
- **Problem:** Each agent start and finish scans every node; months of history take seconds per reduce, and every view reduces the whole log.
- **Fix:** look up a session's own nodes by range, and waits by target.
- **Status:** fixed across sessions. A session's own nodes are found as a range of the node map (`family`), not by looking at every node, when pairing an agent (`bind_by_guess`) or a session (`bind_session_by_guess`) with its request; the nodes that may be waiting on each node are indexed (`waiting_on`), so an end closes their waits directly (`close_waits_on`); and each request's makers are indexed by call id (`requesters`), so undoing a wrong guess finds its request directly. That undo now also clears only the request bound to that child: call ids aren't unique across sessions, and it used to clear another session's pairing too. A history of 1,000 sessions reduces in about 0.3 s rather than about 10 s (debug). Within one session, an agent's start and finish still look at that session's nodes and waits: see R53. Tests: `reducing_grows_in_step_with_the_history` (tests/reducer.rs), timed in the thread's CPU time (so other work on the machine doesn't sway it): 16 times the history takes about 19.5× as long (limit 48×; putting back any of the four scans makes it 108–317×, debug), and every child is still paired, every wrong guess undone and every wait closed; `a_wait_on_a_started_session_ends_with_it_even_if_it_resumes`, `a_wait_ends_with_its_target_even_if_it_resumes`, `a_spawn_wait_left_open_by_a_finish_closes_at_the_end`, `correcting_a_guess_leaves_other_sessions_requests_alone` (each index, and the undo's fix). Reviewed (two rounds).

### R23. Every change, and every timeline step, reduces and sends the whole history
- **Where:** `src/timeline.rs`, `src/view/mod.rs`, `src/view/assets/app.js`
- **Problem:** Each request replays all events and returns every node of every session, with the events lock held; with a large history each step takes about half a second locally and freezes the site's page.
- **Fix:** send only the selected session's tree (plus a summary of the others), and compute outside the lock.
- **Status:** fixed. A graph reply (`timeline::graph`) now carries the tree under the node asked for (`root`, echoed; if none is asked for or it isn't in the graph, the most recently active session's, so the page never needs a second request), `others`: only what names the nodes the tree refers to outside it (what it's waiting on, its messages' peers, the children its requests started when R54 has put them elsewhere, its root's parent), and a summary of every session (`sessions`: what names it, its state, tasks, how many agents, the first node needing you or looking stuck, whether it's deadlocked or busy) for the sidebar. The page asks for the node it shows, judges a reply by the tree it says it holds, ignores one overtaken by a change of session, lists sessions from their summaries, and shows a node outside the tree by switching to its own (at any step), as it does for one named in the address (going back to where it was if that doesn't exist). The local server keeps its events behind an `Arc`, so a request takes a snapshot under the lock and works out its reply without it (and so do `/api/open` and images). The site's WebAssembly, which hadn't been rebuilt since R14, is rebuilt (without the build machine's paths in it), and now records what it was built from (the crate's files the build compiled, from its dep-info; the crates it used, with their versions and features; and its build profile), which CI checks (`sync-viewer.mjs --check-wasm`), so a change to the reducer can't reach the site unbuilt, and a change to only the CLI doesn't need a build. Still to do: R55 (the snapshot is copied when events arrive), R56 (each refresh reduces twice). Tests: `requests_are_worked_out_without_holding_the_events` (src/view/mod.rs), `a_graph_carries_one_tree_and_a_summary_of_every_session`, `only_the_trees_sessions_can_be_opened`, `a_child_claimed_from_outside_the_tree_is_named` (src/timeline.rs), "the site's WebAssembly sends one tree, and every session's summary" (site/tests/wasm.test.mjs), and in site/tests/viewer.test.mjs (whose stubs now reply as the server does: `asServer`) "R23: the page asks for the tree it shows, and lists the others from their summaries", "…with no session named, one request brings the newest session's tree", "…a node outside the tree is named, and opening it shows its own tree", "…an address naming a node this graph doesn't have shows its tree", "…an address naming a node that doesn't exist goes back to where it was", "…at a past step, a node outside the tree still opens its own tree", "…an old session named in the address is shown", "…a refresh overtaken while its graph comes shows nothing of it", "…a node of the tree that hadn't started at a step says so", "…with no session named and none recent, none is shown", and the R7 test's request for the tree. Reviewed (three rounds).

### R24. A shell-launched session can be paired with another program's request, for good
- **Where:** `src/reducer.rs` (`bind_session_by_guess`)
- **Problem:** With no request for its own program, a child takes the oldest open request of any program, including a background one from long ago, and takes its purpose and type.
- **Fix:** don't pair across known, different programs, and only pair background requests made shortly before.
- **Status:** fixed. `bind_session_by_guess` no longer considers a request for an agent CLI the child surely isn't running (`surely_not`: one of the shell adapter's built-in agent CLIs that the child's provider, per `PROVIDER_PROGRAMS`, doesn't run; a wrapper added with `AGENT_GRAPH_AGENT_COMMANDS`, or an `agent-graph run` child, might be it), nor a background request made more than `BACKGROUND_START` (10 minutes: the command can do other things first) before the child started; one made more than `FRESH_START` (a minute) before comes after the rest, since it may never launch anything (a failed launch, or R25's false positives). It takes the first request for the child's own program, or failing that the first that might be it. `adapter::PROVIDERS` lists every adapter's provider, and a test checks each is in `PROVIDER_PROGRAMS`. The site's WebAssembly is rebuilt. Tests: `a_shell_session_isnt_paired_with_another_programs_request`, `only_a_recent_background_request_is_paired_with_a_shell_session` (tests/reducer.rs), `within_needs_both_times`, `every_provider_says_which_programs_it_runs` (src/reducer.rs). Reviewed (three rounds). Still open: R57.

### R25. The shell parser sees launches that aren't
- **Where:** `src/adapter/shell.rs`
- **Problem:** Heredoc bodies (every commit message Claude writes) are parsed as commands, names are matched case-insensitively ("Claude & Codex …"), `command -v claude` counts, and so do other tools called `goose` or `copilot`.
- **Fix:** skip heredoc bodies, match names exactly, treat `command -v` as a lookup, and skip those tools' non-agent subcommands.
- **Status:** fixed. The splitter notes each unquoted `<<WORD` (`<<-`, and quoted WORDs, too; several on a line) and skips the lines after that one up to WORD (tab-indented only for `<<-`, and before a CR), but not in arithmetic (`$((…))`, a `((…))` starting a command, after any keywords), which it skips as a word, when it closes as arithmetic within `ARITHMETIC_MAX` (1 KiB, which keeps a long run of `(` linear) and holds no command substitution (`closes_as_arithmetic`: otherwise it's a command substitution or subshells, of commands), nor `$[…]`; a quoted WORD can hold spaces, or be empty (a blank line ends that body); program names are compared exactly (`named`), except a Windows program named with its extension, which is compared in any case and keeps its own; `command` with an option containing `v` or `V` is a lookup; `goose` counts only with `session`, `run` or `review` (`ONLY_SESSIONS`), and not `session list` and the like (`MANAGES_SESSIONS`, the word right after `session`); `copilot`'s `NOT_SESSIONS` has AWS Copilot's subcommands and the Copilot CLI's management ones, matched only as its first argument (`SUBCOMMAND_FIRST`), so an option's value isn't taken for one; and `agent-graph run` names its sessions with the parser's `program_name`, so the two agree. The site's WebAssembly is rebuilt. Tests: `heredoc_bodies_arent_commands`, `heredocs_are_read_as_the_shell_reads_them`, `arithmetic_isnt_a_heredoc`, `names_are_matched_exactly`, `windows_programs_keep_their_names`, `command_lookups_arent_launches`, `other_tools_with_agents_names_arent_launches`, `management_commands_arent_launches` (src/adapter/shell.rs), `names_come_from_the_program` (src/run.rs). Reviewed (three rounds).

### R26. The shell parser misses launches after `"$(…)"` and line continuations
- **Where:** `src/adapter/shell.rs`
- **Problem:** A substitution inside double quotes flips the quote state for the rest of the command, and a backslash-newline becomes a word.
- **Fix:** track quotes per substitution level; drop backslash-newlines.
- **Status:** fixed. The splitter reads a group (`( … )`) or command substitution (`$( … )`, backticks) as a level of its own (`Level`), which starts with no quotes; at its end (`)`, unless it ends a `case` pattern at that level, or the closing backtick) the command around it carries on as it was, in its quotes, with the substitution as (part of) a word, so `codex exec "$(cat prompt.md)" &` is a background launch, `"$(pwd)/bin/claude"` runs claude, and `echo $(date) codex` doesn't run codex; one left open ends with the command. Commands come out in the order the shell runs them (a substitution before the command it's in). So that `{` isn't a separator any more, a function's definition (`NAME()`, an empty group after a name alone, or `function NAME`) ends there, and its body is a command of its own (`claude -p hi <()` is still a command), and `xargs` and `parallel` are wrappers, whose options' values (as for the other wrappers: `nice -n 10`, `sudo -u bot`, `timeout -s KILL`, `xargs -I {}`, and in a cluster, `xargs -tP 4`) aren't taken for the program (`WRAPPER_VALUES`, `takes_value`). A backslash-newline is dropped (and before a CR, which bash would keep); a comment is skipped (it can hold an apostrophe; in backticks, it ends at the one that ends them); `$'…'` is read with its escapes, but not after `$$`; `${…}` is one word, spaces and all, and `{` on its own is a keyword. Whether a word starts a command is kept as a count of the keywords (and `function NAME`) before it (`Level::lead`), so a long run of them doesn't take quadratic time. The site's WebAssembly is rebuilt. Tests: `quotes_carry_on_after_a_substitution`, `a_substitution_is_a_word`, `line_continuations_join_lines`, `comments_are_skipped`, `other_quotes_are_read_as_the_shell_reads_them`, `functions_bodies_are_commands`, `wrappers_option_values_arent_programs`, `splitting_takes_time_in_proportion_to_the_command`, and in `arithmetic_isnt_a_heredoc` (src/adapter/shell.rs). Reviewed (three rounds). Still open: R58, R59, R60.

### R27. A long TodoWrite list is dropped entirely
- **Where:** `src/emit.rs` (`to_line`)
- **Problem:** A list over 4 KB (about 18 ordinary items) becomes an `unknown` event, so the task list and headline go stale.
- **Fix:** shrink it (shorter text, then fewer items, marked) so it keeps its type.
- **Status:** fixed, keeping it whole instead: a list is one piece of state, which splitting it into several events would show as many steps on the timeline (and a partial list at each), so its line may take `MAX_TASKS_LINE` (16 KB, about a hundred ordinary items; other events keep `MAX_LINE`, 4 KB), and past that `to_line` cuts its items' text shorter, in turn (`TASK_TEXT_CUTS`: 200, 80, 40, 20 characters, then 1), leaving ids and statuses whole, and the first item in progress, which is the headline, no shorter than 200. A list that doesn't fit even so (more than about 240 items) is still recorded as `unknown`. The cap is kept small because a todo tool sends its whole list at every change: working through a 240-item list writes about 7.5 MB, well within the site's 64 MiB per log, and a 16 KB line is well within what the uploader sends in a chunk (256 KB) and the site accepts (512 KB). Tests: `long_task_lists_are_kept_whole`, `events_too_long_even_shortened_are_recorded_as_unknown` (src/emit.rs). Reviewed (four rounds; the first design, splitting a list into several events, was dropped).

### R28. A status sorted after `session.ended` brings the session back to life
- **Where:** `src/reducer.rs`
- **Problem:** A headless session's Stop and SessionEnd can land in the same millisecond in either order; if Stop sorts last, the session shows as idle (alive) forever.
- **Fix:** only a new `session.started` revives an ended session.
- **Status:** fixed. The reducer notes when each session ends (`Reducer::ended`), until it starts again, and ignores a status for it or its agents less than `LATE_STATUS` (2 s) after that, unless it's a terminal one (which still says how it ended: failed, say); an agent first seen in a late status is canceled with the session. A later status is taken as activity, as before: another process can carry on the same conversation after one exits. After an end less than 2 s after the session's last start (`Reducer::started`), only an idle status (a Stop) is late: the end may be a quick run's own, or the last run's, landing as a resumed run starts, whose first prompt then counts. The statuses ignored are listed (`Graph::late`), and the timeline labels them as late rather than, say, as needing you. The site's WebAssembly is rebuilt. Tests: `a_late_status_doesnt_revive_an_ended_session` (tests/reducer.rs), `a_late_status_is_labelled_so` (src/timeline.rs). Reviewed (four rounds).

## Low

### R29. A `--command` without the usual form duplicates hooks and can't be uninstalled
- **Where:** `src/install.rs`
- **Problem:** Agent Graph finds its hooks by `emit --provider claude-code` in their command; a `--command` without it was installed anyway, so each install added another set, and uninstall left them.
- **Status:** fixed. `install_claude_code` refuses such a command before changing anything, and says why; the `--command` help says it must contain it. Test: `a_command_that_cant_be_found_again_is_refused` (src/install.rs). Reviewed.

### R30. `uninstall --scope local` deletes the project's skill
- **Where:** `src/slash.rs`, `src/cli.rs`
- **Problem:** There's nowhere uncommitted for a project's command, so `--scope local` puts it in the project's folder, like `--scope project`; uninstalling locally deleted the one the project had committed.
- **Status:** fixed. Uninstalling with `--scope local` leaves the command if it's in the repository's last commit (`git ls-tree HEAD`, ignoring `GIT_DIR` and the like from the environment), and says to remove it with `--scope project`; if git can't tell (a repository someone else owns, say), it's kept too, with git's message. Staged, uncommitted, or outside a repository, it's removed. The final line no longer says nothing's installed when something was left alone. Help updated. Test: `local_uninstall_leaves_the_projects_committed_command` (tests/cli.rs). Reviewed (three rounds).

### R31. `snapshot --session X --out x.json` writes every session
- **Where:** `src/cli.rs`
- **Problem:** As JSON, a snapshot ignored `--session` and `--all` and wrote the whole graph.
- **Status:** fixed. With `--session` or `--all`, the JSON is cut down (`cli::only`) to whole trees: the sessions chosen (an agent gives its session), those they exchanged messages with (one step, so a teammate doesn't bring the whole team), and whatever those wait on, directly or not, so the waits and `blocked` counts in the file refer to nodes in it. With neither, it's the whole graph, as before. Asking for sessions with none recorded fails as a picture does. Help updated. Tests: `a_snapshot_of_some_sessions_holds_what_they_name` (tests/sessions.rs), `snapshot_json_is_the_sessions_asked_for` and `snapshot_json_of_sessions_needs_some` (tests/cli.rs). Reviewed (three rounds, and a fourth on the help's wording, stopped before it answered).

### R32. Two snapshots in the same second overwrite each other
- **Where:** `src/cli.rs`
- **Problem:** `snapshot` without `--out` named its picture to the second and wrote it with `fs::write`, so a second snapshot in the same second replaced the first.
- **Status:** fixed. The picture is created new (`store::write_new`), taking the next free `-2`, `-3`… suffix when the name is taken. Test: `a_snapshot_never_replaces_one_from_the_same_second` (tests/cli.rs). Reviewed.

### R33. U+FFFE or U+FFFF in any text breaks every image of that session
- **Where:** `src/image.rs` (`escape`)
- **Problem:** XML forbids U+FFFE and U+FFFF, and `escape` passed them through, so one in any text made usvg reject the whole SVG, and every picture of that session failed.
- **Status:** fixed. `escape` turns them into spaces, as it did control characters. Test: `escapes_characters_xml_forbids` (src/image.rs). Reviewed.

### R34. The per-step graph cache grows without limit
- **Where:** `src/view/assets/app.js`
- **Problem:** The per-step graph cache kept a graph for every step visited and only started afresh at each refresh, so on a pasted log, dragging across a long timeline piled up graphs for as long as the page was open.
- **Status:** fixed. It keeps the 32 most recently shown steps (`CACHED_STEPS`, `remember`). Test: "R34: only the most recently shown steps' graphs are kept" (site/tests/viewer.test.mjs). Reviewed.

### R35. Re-rendering scrolls the tree back to the ringed card
- **Where:** `src/view/assets/app.js`
- **Problem:** `renderMain` scrolled the ringed card into view every time it drew, so choosing a card, or a live update while looking at a past step, undid the reader's scrolling.
- **Status:** fixed. The ringed card is brought into view only when what it stands for changes (the session, the step, or the node it touched: `S.scrolledTo`), once it's actually drawn, so one whose node isn't in the graph still shown is scrolled to when its step's graph comes. Following live rings nothing, and clears the record. Tests: four "R35: …" tests (site/tests/viewer.test.mjs). Reviewed (three rounds).

### R36. A malformed URL hash blanks the viewer
- **Where:** `src/view/assets/app.js` (`rootFromHash`)
- **Problem:** `hashId` (was `rootFromHash`) called `decodeURIComponent` unguarded, so an address with a bad %-escape made every refresh fail and the viewer stayed blank.
- **Status:** fixed. An address that doesn't decode names nothing: the newest session is shown, and a malformed hash change does nothing. Test: "R36: an address that isn't a well-formed one names nothing, and the page still works" (site/tests/viewer.test.mjs). Reviewed.

### R37. Long unbroken text overflows cards and panels
- **Where:** `src/view/assets/app.css`
- **Problem:** Text with nowhere to break (long titles, folders, purposes, headlines, message bodies) widened cards and panels.
- **Status:** fixed. `.layout` has `overflow-wrap: anywhere`, inherited by the sessions list, main view and details. Test: "R37: long unbroken text wraps in cards and panels" (site/tests/viewer.test.mjs, app.css in jsdom). Reviewed.

### R38. Live updates drop keyboard focus and text selection
- **Where:** `src/view/assets/app.js`
- **Problem:** Each live update rebuilt the sessions list, the tree and the details with `replaceChildren`, so keyboard focus went back to the page and a text selection was lost.
- **Status:** fixed. Redraws change the elements already there in place (`morph`, `redraw`), carrying over text, attributes and click handlers; a focused control that now stands for something else, or has gone, hands focus to the one for the same node (`data-id`). Test: "R38: new events leave keyboard focus and selected text where they were" (site/tests/viewer.test.mjs). Reviewed.

### R39. The timeline's hover tip shows labels without cleaning them
- **Where:** `src/view/assets/app.js` (`showTip`, `aria-valuetext`)
- **Problem:** `showTip` and `aria-valuetext` used a stop's label uncleaned: not an injection (it's set as text), but control characters and bidirectional overrides were shown and spoken, unlike everywhere else.
- **Status:** fixed. Both pass it through `clean()`. Test: "R39: the timeline's hover tip and spoken step show a label as text, cleaned" (site/tests/viewer.test.mjs). Reviewed.

### R40. Snapshot layout: the stale flag overlaps, deep trees get negative widths, agents are uncapped
- **Where:** `src/image.rs`
- **Problem:** The snapshot's "stale?" flag had no room kept for it (it ran over the "background" label, the task count and the header's margin); each tree level was indented 22 more pixels, so past about 22 levels cards got negative widths; and every agent and callout was drawn, so a session of 5,000 agents gave a 5 MB SVG.
- **Status:** fixed. Names and titles are cut to leave room for the pill and flag (`pill_width`) and for what's right-aligned, which is measured. Every level is indented, with a smaller step for trees deeper than six, at most 132 pixels in all (`MAX_INDENT`), and connectors run beside cards, not through them. A session draws its first 30 agents (`MAX_AGENTS`, `tree_order`) and 3 callouts (`MAX_CALLOUTS`), then "+N more agents" and "+N more need you". Help updated. Tests (src/image.rs): `the_stale_flag_has_room`, `a_deep_tree_stays_inside_the_picture`, `a_deep_tree_keeps_its_shape`, `a_huge_session_draws_some_agents_and_counts_the_rest`, `agents_past_the_cap_are_counted_exactly`, sharing a layout checker (`assert_laid_out`). Reviewed (two rounds).

### R41. Session requests only pair at a child's first start
- **Where:** `src/reducer.rs`
- **Problem:** A child resumed from the parent's shell, or one whose start sorts just before the request, is never paired.
- **Status:** open

### R42. `claude … & claude … & wait` counts as a background launch
- **Where:** `src/adapter/shell.rs`
- **Problem:** `claude … & claude … & wait` counted as a background launch, but the call doesn't return until the jobs finish, so the requester is waiting on them.
- **Status:** fixed. Each shell (the top, a subshell or a substitution) keeps the jobs it put in the background (`Level::jobs`), and a `wait` it runs itself takes back those started before it. Not counted: `wait -n`; a `wait` in a pipeline; one whose list a `&` ends; one in a function's body (any compound command after `NAME()` or `function NAME`). Known gap: a `wait` inside a compound that's then put in the background (`{ wait; } &`) still counts. Test: `jobs_waited_for_arent_in_the_background` (src/adapter/shell.rs). Reviewed (three rounds).

### R43. A node id with thousands of `/` overflows the stack
- **Where:** `src/reducer.rs` (`ensure`)
- **Status:** open

### R44. The log size cap doesn't bound storage
- **Where:** `site/app/api/logs/[id]/append/route.ts`
- **Problem:** The 64 MiB cap was judged from offsets, so it bounded a log's span, not what it stored: chunks at overlapping offsets were each stored in full (only a log's own writer could do this).
- **Status:** fixed. Each log counts the bytes its chunks hold (`stored`, the sum of their lengths `n`), in the append's transaction and each trim batch's; an append past `MAX_LOG_BYTES` gets 413 "This log is full" (a retry of a chunk already stored excepted). The count stays exact across the storage-id migration, which merges its metadata, never replaces a chunk, and counts uncounted chunks in transactions. Tests: "overlapping chunks count towards the size limit" (site/tests/api.test.mjs), "a log counts what it stores, and trimming gives it back" and "the size count is exact across the migration" (site/tests/store.test.mjs). Reviewed (two rounds).

### R45. Create and unlock buffer the whole body when there's no Content-Length
- **Where:** `site/app/api/logs/**`
- **Problem:** Create and append checked Content-Length, but a body without one was read whole before being measured; unlock had no limit at all.
- **Status:** fixed. `bodyText(req, max)` reads the body as it arrives and stops past `max`: 512 KB for create and append, 8 KB for unlock (`UNLOCK_BODY_BYTES`), answering 413 past them. Unlock strips a leading byte-order mark before parsing, as `req.json()` did. Tests: in site/tests/unlock.test.mts and "bad input is refused" (site/tests/api.test.mjs). Reviewed (two rounds).

### R46. Passwords of 1025–1050 bytes are accepted but can never unlock
- **Where:** `site/lib/crypto.ts`, unlock route
- **Problem:** Creating a log allowed a 1050-byte password, but unlocking refused any over 1024 UTF-16 units, so some passwords could be set and never used; the CLI had no limit.
- **Status:** fixed. One limit, `MAX_PASSWORD_BYTES` (1024 bytes of UTF-8, as hashed), for creating (400 past it), unlocking, and the CLI (`check_password`, before sending). Tests: "a password a log accepts can unlock it, and one it refuses can't" (site/tests/unlock.test.mts), in "bad input is refused" (site/tests/api.test.mjs), `a_password_too_long_for_the_site_is_refused` (tests/remote.rs). Reviewed.

### R47. The CLI's refusal to send a password over plain HTTP can be bypassed
- **Where:** `src/remote.rs`
- **Problem:** The check parsed the URL by hand: an uppercase scheme, a userinfo password (`http://localhost:x@evil.example`) or any IPv6 literal starting `[::` passed as private, and redirects were followed with the password header.
- **Status:** fixed. `sends_privately` judges the URL as the HTTP client parses it (`https`, or `http` to localhost or a loopback address), no redirects are followed, and plain HTTP to this machine skips any proxy. Tests: `a_password_is_only_sent_over_https_or_to_this_machine` (src/remote.rs), `a_redirect_is_not_followed` (tests/remote.rs). Reviewed.

### R48. `tail` leaves the terminal broken when killed
- **Where:** `src/live.rs`
- **Problem:** In raw mode Ctrl+C is a key press, but SIGTERM, SIGHUP or SIGINT from elsewhere killed `tail` without restoring the terminal (left raw, on the alternate screen, cursor hidden).
- **Status:** fixed. On Unix, `tail` catches them once each (unless started with them ignored), with `SA_RESETHAND` so a second still kills one stuck writing; the first stops the loop, restores the screen, and is raised again so the sender sees it die of it. Tests: `tail_puts_the_terminal_back_when_killed` (tests/cli.rs, on a pseudo-terminal), `a_second_signal_is_the_default` (src/live.rs). Reviewed (three rounds).

### R49. Every install or uninstall overwrites the backup, losing the original
- **Where:** `src/cli.rs`
- **Problem:** Every install or uninstall copied the current settings over the backup, so after a reinstall or uninstall it held Agent Graph's hooks, not the user's original.
- **Status:** fixed. Only a settings file without Agent Graph's hooks is backed up. Help updated. Test: `the_settings_backup_keeps_the_original` (tests/cli.rs). Reviewed.

### R50. On Windows, `run` turns exit codes above 255 into 1
- **Where:** `src/run.rs`
- **Problem:** `run` exited with the child's code as a byte, or 1, so on Windows any code above 255 (a crash's 0xC0000005, say) became 1.
- **Status:** fixed. `exit_with` picks the exit: a code that fits a byte as it is; on Windows any other passed whole via `process::exit`, after the run's recorded. Test: `windows_exit_codes_pass_through_whole` (src/run.rs). Reviewed.

### R51. `uninstall` leaves behind the empty folders `install` created
- **Where:** `src/cli.rs`
- **Problem:** Uninstall left the empty `.agents/`, `.gemini/`, `.cursor/` folders install made, and `.claude/` holding a `settings.json` of `{}`.
- **Status:** fixed. Uninstall also removes an agent folder left empty, and a settings file left as `{}` unless there's a backup beside it or it's a link. The help says exactly this. Test: `uninstall_leaves_no_empty_folders_behind` (tests/cli.rs). Reviewed (two rounds).

### R52. Creating a log with a password, or unlocking one with the right password, runs scrypt without limit
- **Where:** `site/app/api/logs/route.ts` (`hashPassword`), `site/lib/unlock.ts`
- **Problem:** Found reviewing R19. Only wrong guesses are limited, so one address can run scrypt (about 50 ms each) as often as it likes by unlocking its own log with the right password, or by creating logs with passwords, which isn't limited at all.
- **Fix:** a generous per-address cap on every scrypt run (say 200 per 15 minutes), counted like R19's buckets.
- **Status:** fixed. Every scrypt run is counted, never given back, in buckets like R19's (the same TTL policy), per 15 minutes: 200 password checks at one log from one address (`SCRYPT_CHECKS_PER_LOG_AND_ADDRESS`), so a log's viewers behind one address hold back only that log there; and 2000 runs from one address in all (`SCRYPT_RUNS_PER_ADDRESS`), checks and logs created with a password, the bound on the server's time. Past either, 429 with `Retry-After`. Tests: "checking passwords at a log is limited per address" (site/tests/api.test.mjs), "scrypt runs are counted per address, and per log and address" (site/tests/store.test.mjs). Reviewed (two rounds).

### R53. Within one session, the reducer is still quadratic in its agents
- **Where:** `src/reducer.rs` (`bind_by_guess`, the per-node `spawns` and `waits` lists)
- **Problem:** Found reviewing R22. An agent's start looks at every node of its session for an unpaired request, and a finish looks through the waiting node's waits; a session with thousands of agents (or of sessions started from it) takes seconds to reduce (4,000 agents: about 1.9 s, debug).
- **Fix:** index each session's unpaired requests, and waits by what they wait on.
- **Status:** open

### R54. A child named by another session's request with the same call id is claimed by both
- **Where:** `src/reducer.rs` (`bind`)
- **Problem:** Found reviewing R22. `spawned_by` holds only a call id, not the node that made the request, so when `spawn.returned` names a child already bound to a request with the same call id in another session, `bind` doesn't undo the first binding, and both requests claim the child. Unlikely (call ids are almost always unique), but possible.
- **Fix:** record the requester with the call id, and compare both.
- **Status:** open

### R55. A poll copies the whole event log while a request holds a snapshot
- **Where:** `src/view/tail.rs`
- **Problem:** Found reviewing R23. The events are an `Arc<Vec<Timed>>`; when a poll finds new ones while a request still holds a snapshot, `Arc::make_mut` copies every event, under the lock (about 12 ms, release, for 32,800 events), and holds a third copy of the log meanwhile.
- **Fix:** keep each event behind its own `Arc` (`Vec<Arc<Timed>>`), so the copy is of pointers.
- **Status:** open

### R56. Each refresh reduces the history twice, on the site in the page's main thread
- **Where:** `src/view/assets/app.js` (`refresh`), `site/public/viewer/site-source.js`
- **Problem:** Found reviewing R23. A refresh asks for the graph and then the timeline, and each reduces every event; on the site, that's in the page's main thread (about 130 ms each for 32,800 events), so the page stalls on busy logs.
- **Fix:** one request for both (reducing once), and run the WebAssembly in a Worker on the site.
- **Status:** fixed. A graph request for now also returns its tree's timeline (`stops`), from the same reduction, so a refresh is one request; `/api/timeline` and the WebAssembly `timeline` op are gone. On the site, `site-worker.js` reads the log and runs the WebAssembly in a Worker, and `site-source.js` talks to it by messages. Tests: `the_graph_now_carries_its_trees_timeline` (src/timeline.rs), `the_graph_endpoint_carries_the_timeline` (tests/view.rs), and in site/tests (wasm, viewer and site-source tests, the last with a stand-in Worker). Reviewed.

### R57. A named or wrapped `agent-graph run` can take another program's request
- **Where:** `src/reducer.rs` (`bind_session_by_guess`), `src/run.rs`, `src/adapter/shell.rs`
- **Problem:** Found reviewing R24. A run session is named for the run (`--name workers`) or the wrapper it was given (`npx`), not the agent CLI, so it might answer any request, including one for a CLI with no adapter (`codex exec … &`, which nothing else ever claims): it then takes that request's program, purpose and `background`, for good, and its own request shows as starting for the whole run.
- **Fix:** record which requests were for `agent-graph run` (the shell adapter knows), and pair a run only with those, and nothing else with them; or have `run.rs` record the program it wraps, found as the shell adapter finds it.
- **Status:** fixed. The shell adapter says when a launch is through `agent-graph run`, and the Claude Code adapter records it on every session request (`spawn.requested`'s `run`). A run's session pairs only with a request for a run, and no other session with one; a request from a log written before `run` was recorded pairs as before (with a run's session only if the run's named for its program). Example logs regenerated. Tests: `runs_say_they_are_runs` (src/adapter/shell.rs), `a_run_is_paired_only_with_a_request_for_a_run`, `a_run_is_paired_with_an_old_logs_request_by_its_name` (tests/reducer.rs), `a_run_answers_the_request_made_for_it` (tests/sessions.rs). Reviewed (two rounds).

### R58. `&` puts only the last simple command in the background
- **Where:** `src/adapter/shell.rs`
- **Problem:** Found fixing R26. A trailing `&` backgrounds the whole and-or list or group before it, but the splitter marks only the last simple command, so `(cd x && codex exec y) &`, `{ codex exec y; } &`, `claude -p a && echo done &` and `claude -p a | tee log &` are foreground launches; `0<&3` is read as `&`, putting the command before it in the background; and an array assignment, `arr=(claude codex)`, is read as a group, so it's a launch.
- **Fix:** mark everything since the list began (with its groups and substitutions) when a `&` ends it, keep `<&` in its word, and read `NAME=(…)` as a word.
- **Status:** fixed. A lone `&` puts everything since its list began in the background (`Level::list`), a compound command counting as one command of its list; `<&`, and the `&` of `|&` (a pipe), stay in their words; `NAME=(…)` is read as an array's values. Test: `a_background_list_is_all_in_the_background` (src/adapter/shell.rs). Reviewed (two rounds).

### R59. A `case` pattern after the first is read as a command
- **Where:** `src/adapter/shell.rs`
- **Problem:** Found reviewing R26. In `case $a in claude) …;; codex) …;; esac`, the patterns after `;;` (or on a line of their own) come out as simple commands, so `codex` is a launch that isn't there.
- **Fix:** after `in` and after each `;;` (`;&`, `;;&`) in an open `case`, read the words up to `)` as a pattern and drop them.
- **Status:** fixed. After `case WORD in`, and after each `;;`, `;&` or `;;&`, the words up to `)` are a pattern and dropped (across newlines, `|` alternatives and a leading `(`); `esac` where a pattern would start closes the `case`. Test: `case_patterns_arent_commands` (src/adapter/shell.rs). Reviewed.

### R60. A heredoc ended by `EOF)` inside a substitution swallows the rest of the command
- **Where:** `src/adapter/shell.rs`
- **Problem:** Found reviewing R26. Bash ends a heredoc inside `$( … )` (or backticks) at a line that's the delimiter followed by the `)` (or backtick) that closes it; the splitter only ends one at a line that's the delimiter alone, so the rest of the command is taken as the body.
- **Fix:** inside a substitution, end the body at the delimiter followed by its closing `)` or backtick, and read on from there.
- **Status:** fixed. Inside a substitution, a body line that's the delimiter followed by that substitution's closer ends the heredoc, and reading resumes at the closer (as bash does). Test: `a_heredoc_can_end_with_its_substitution` (src/adapter/shell.rs). Reviewed.

### R61. Upgrading through a package manager breaks the hooks
- **Where:** `src/install.rs` (`default_command`)
- **Problem:** Found setting up packaged releases. Hooks in your own settings run this copy by its full path, and on Linux that's the file a link leads to: under Homebrew, `Cellar/agent-graph/<version>/bin/agent-graph`, which the next upgrade removes, so every hook fails until `install` is run again. On macOS it's whatever path it was run by, which can be the same.
- **Status:** fixed. When the `agent-graph` on PATH is this copy, the hooks name it by that path (Homebrew's `bin`, WinGet's `Links`), which upgrades keep; otherwise this copy's own path, as before. Test: `user_hooks_name_the_link_on_path_not_the_versioned_file` (tests/cli.rs).
