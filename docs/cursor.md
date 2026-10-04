# Cursor support: plan

**Built so far (not yet released):** the local adapter (`src/adapter/cursor.rs`), checked against a real Cursor (step 1, the approvals run, and a live run, below), with tests on its payloads (`tests/cursor.rs`). `emit` answers Cursor's hooks with `{}`. Sessions a chat starts link to it through `CURSOR_CONVERSATION_ID`. A subagent is paired with the exact `Task` call that started it (`agent.spawned`'s `call_id`), and one still at work when its turn is stopped is canceled. "Needs you", as far as Cursor's hooks allow: a command outside the sandbox that's gone quiet, the CLI's allowlist, a reply ending on a question, and a refused command (see "Needing you"). `install cursor` writes the hooks, but stays hidden until it's released. Not built yet: resuming a chat, the site, and the cloud.

Agent Graph records Claude Code's and Codex's sessions through their hooks. This is the plan for recording Cursor's the same way. Locally, that's the Cursor app's agent and the Cursor CLI (`cursor-agent`, also installed as `agent`). In the cloud, it's Cursor's cloud agents, which are started from cursor.com/agents, the app, the phone app, Slack, GitHub, Linear or the API.

Written 2026-10-04 from Cursor's docs ([hooks](https://cursor.com/docs/agent/hooks), [subagents](https://cursor.com/docs/agent/subagents), [cloud agents](https://cursor.com/docs/cloud-agent), [cloud agents API](https://cursor.com/docs/cloud-agent/api/endpoints), [CLI output format](https://cursor.com/docs/cli/reference/output-format)) and Cursor's forum. The current versions are app 3.x and CLI builds like `2026.09.18`. None of it has been checked against a real Cursor yet, and Cursor isn't installed on the development Mac. Step 1 of each half is that check, and it comes before building.

## What Cursor gives us

Cursor's hooks are the same idea as Claude Code's and Codex's: a command, with JSON on stdin. Its names and shapes are its own, though.

- **Where.** These are merged, highest priority first:
  - enterprise: `/Library/Application Support/Cursor/hooks.json`, `/etc/cursor/hooks.json` or `C:\ProgramData\Cursor\hooks.json`;
  - team, from Cursor's dashboard;
  - a project's `.cursor/hooks.json`, run from the project's root;
  - the user's `~/.cursor/hooks.json`, run from `~/.cursor/`.

  The format is `{"version": 1, "hooks": {"<event>": [{"command", "timeout", "matcher", "failClosed"}]}}`, with camelCase event names. As far as the docs say, there is no trust step like Codex's.
- **Exit codes.** 0 is fine, and 2 blocks. Any other code fails open, unless `failClosed` is set.
- **Output.** Cursor reads a hook's stdout as JSON. On Windows, the CLI runs hooks through PowerShell and [takes empty stdout for invalid JSON](https://github.com/leeguooooo/AgentParty/issues/1123), which blocks the tool. So `emit --provider cursor` must print `{}`: the opposite of Codex, where `emit` must stay silent.
- **Events.** `sessionStart` and `sessionEnd`; `beforeSubmitPrompt`; `preToolUse`, `postToolUse` and `postToolUseFailure`; `subagentStart` and `subagentStop`; `beforeShellExecution` and `afterShellExecution`; `beforeMCPExecution` and `afterMCPExecution`; `beforeReadFile` and `afterFileEdit`; `preCompact`; `stop`; `afterAgentResponse` and `afterAgentThought`. There are also Tab and app events we don't need.
- **Common fields.** `conversation_id`, `generation_id` (one per turn), `hook_event_name`, `model`, `cursor_version`, `workspace_roots`, `user_email` and `transcript_path`. Hooks also get `CURSOR_PROJECT_DIR`, `CURSOR_VERSION`, `CURSOR_TRANSCRIPT_PATH` and `CURSOR_CODE_REMOTE` in their environment.
- **Per event**, what matters to us:
  - `sessionStart`: `session_id`, `is_background_agent` and `composer_mode`. It can return `env`, variables for the session's commands. That's the equivalent of Claude Code's env file, and how a session can pass `AGENT_GRAPH_PARENT` to the sessions it starts.
  - `sessionEnd`: `reason` (completed, aborted, error, window_close or user_close) and `final_status`.
  - `subagentStart`: `subagent_id`, `subagent_type`, `task`, `parent_conversation_id`, `tool_call_id`, `is_parallel_worker` and `git_branch`. This is the best parent data of any provider.
  - `subagentStop`: `status` (completed, error or aborted), `task`, `summary` and `agent_transcript_path`. It has **no `subagent_id`**.
  - `preToolUse` and `postToolUse`: `tool_name`, `tool_input`, `tool_use_id` and `cwd`; `postToolUse` adds `tool_output`. The tool types are `Shell`, `Read`, `Write`, `Grep`, `Delete`, `Task` (a subagent) and `MCP:<tool>`.
  - `stop`: `status` (completed, aborted or error).
- **Subagents** are started with the `Task` tool. Several run in parallel, in the foreground or background, nested at most one level deep. The built-in ones are Explore, Bash and Browser, and custom ones live in `.cursor/agents/`. Background ones keep state in `~/.cursor/subagents/`.
- **Transcripts** are JSONL in about Anthropic's message shape, at `~/.cursor/projects/<project>/agent-transcripts/<id>/<id>.jsonl`, with subagents' in `subagents/` beside it. Hooks name the path. This is known only from third-party tools, not from Cursor's docs.
- **The CLI** fires the same hooks. `agent -p --output-format stream-json` prints NDJSON (`system/init`, `assistant`, `tool_call started|completed`, `result`), but we don't need it.

### What Cursor doesn't give us

- **No event says it needs you.** There is no `Notification` or `PermissionRequest`, and no hook is documented for Cursor asking to run a command. Asking a question (`AskQuestion`) or waiting on plan approval (`CreatePlan`) fires no hooks at all. Plan mode even skips `preToolUse`. Cursor's staff [called this a bug](https://forum.cursor.com/t/cursor-cli-askquestion-tool-skips-pretooluse-and-posttooluse-hooks/161836) in May 2026, and it was still there in July.
- **Subagent hooks are unreliable.** A background subagent's `subagentStop` [doesn't fire](https://forum.cursor.com/t/166681), and on 3.11.19 subagent hooks [went silent until a restart](https://forum.cursor.com/t/subagentstart-and-subagentstop-hooks-never-fire-foreground-or-background-while-beforeshellexecution-from-the-same-hooks-json-works-normally/168758).
- **No plan or todo event is documented.** Cursor's agent keeps a todo list, so its tool probably reaches `postToolUse`; step 1 finds its name.

## Local

### Step 1: find out (you, about 20 minutes)

This is done on your Mac, with Cursor installed and signed in (cursor.com/downloads), and the CLI too (`curl https://cursor.com/install -fsS | bash`).

1. In this repository, run `scripts/cursor-probe.sh install`. It points every hook in `~/.cursor/hooks.json` at itself, keeping a copy of any `hooks.json` already there. Each hook appends its payload to `~/.agent-graph-probe/cursor.jsonl`, and the Cursor variables in its environment to `cursor-env.txt` beside it (only the names of anything that might be a key or token). It also prints `{}`, and on `sessionStart` it sets `AGENT_GRAPH_PROBE_ENV` for the session's commands.
2. **In the Cursor app**, open any repository you don't mind a throwaway chat in, and give the agent each of these as a new chat, answering whatever it asks:
   - "Make a todo list of three steps for tidying this README, then do the first."
   - "Use two subagents in parallel: one lists the files in src, one counts lines in README. Then summarize." Then the same again, asking for them in the background.
   - "Run `npm view left-pad version`." Leave Cursor asking to run it for a minute before approving, so we can see what fires while it waits.
   - In Plan mode: "Plan how to add a CHANGELOG, and ask me one question before you do."
   - "Run `claude -p 'say hi'`." This is a session starting another session.
   - "Run `env | grep -E 'CURSOR|AGENT_GRAPH' | cut -d= -f1`." That shows which variables Cursor gives its commands, by name only.
3. **In a terminal**, in the same repository, run `agent` and repeat the first and fourth prompts. Then run `agent -p "list the files here"`.
4. Run `scripts/cursor-probe.sh uninstall`, which puts your `hooks.json` back. Look over `~/.agent-graph-probe/cursor.jsonl` and `cursor-env.txt` (the payloads hold the prompts you typed, and your email), then send them to me.

That shows:

- each event's real payload, which become the test fixtures;
- the todo tool's name and shape;
- what fires around a command waiting for approval, and around a question or a plan waiting for you, if anything;
- what the shell commands' environment carries (any `CURSOR_*` variable naming the conversation);
- whether subagent hooks fire, and which `conversation_id` a hook inside a subagent carries (its own, or its parent's);
- whether `sessionStart`'s `env` reaches the session's commands (`AGENT_GRAPH_PROBE_ENV`);
- how to resume a chat (`agent --resume <id>`, and whether the app has a `cursor://` link to a chat).

### What step 1 found (2026-10-04)

Run in the app (3.23.12) and the CLI (2026.10.01), by `scripts/cursor-probe.sh`. Where Cursor's docs and Cursor differ, Cursor wins:

- **`subagentStart` and `subagentStop` both carry `subagent_id`**, which is the `Task` call's id (the same as `tool_call_id`). `subagentStop` also has `child_conversation_id`, and `agent_transcript_path` is null. No `postToolUse` came for either `Task` call.
- **Ids can hold a newline**: `call-<uuid>-11`, then a newline, then `fc_<uuid>`. Node ids use the first line, which is unique within a chat.
- **Hooks inside a subagent** carry the subagent's own `conversation_id` and `parent_tool_call_id` (its `Task` call), but nothing naming the chat it's in, no transcript path, and none of `sessionStart`'s `env`. A hook can't remember one hook to the next, so they're left out: a subagent shows as working from its start to its stop.
- **`sessionStart`'s `env` reaches the later hooks, but not the commands Cursor runs.** The commands do have **`CURSOR_CONVERSATION_ID`** (and `CURSOR_AGENT`), which is how a session they start links to the chat. Hooks also get `CURSOR_PROJECT_DIR`, `CURSOR_VERSION`, `CURSOR_TRANSCRIPT_PATH` and `CURSOR_USER_EMAIL`, and in the CLI, `CURSOR_INVOKED_AS`. Nothing of Claude Code's environment leaked into Cursor's.
- **Sending a message while a turn is running stops the turn**: `stop` with `status: aborted`, then `beforeSubmitPrompt`. A subagent still at work then hears nothing more, so the reducer cancels a foreground subagent whose call never returned when its parent's turn ends.
- **Hooks run in parallel** (the probe's own log interleaved twice). `emit` appends safely.
- **No hook fired for the todo list, the plan or the question.** Plan mode wrote its plan to `~/.cursor/plans/<name>.plan.md` without a hook, and no `CreatePlan` or `AskQuestion` came.
- **Cursor asked for no approval**, so whether one fires a hook is still unknown. Commands ran in Cursor's sandbox (`sandbox: true`), and `claude -p` outside it (`sandbox: false`), all without asking. The near two-minute gap before `claude -p` was the plan prompt's question, waiting for you, which fired no hook. Checking approvals needs a run with Cursor set to ask before running commands. The timer in "Needing you" isn't built until then.
- **The CLI:** an interactive `agent` sends no `sessionEnd` when you quit. `agent -p` sends `sessionStart`, its tools and `sessionEnd`, with no `beforeSubmitPrompt` and no `stop`. Its chats are kept in `~/.cursor/chats/`, and `agent --resume <chat id>` resumes one.

### The approvals run (2026-10-04)

Cursor set to ask before every command (Run Mode: Allowlist, with nothing on the list), in the app and the CLI. `preToolUse` and `beforeShellExecution` come **before** Cursor asks; `afterShellExecution` and `postToolUse` once you've approved and it has run, and their `duration` includes your wait (23 s for `date`). The CLI's model ran both commands as one (`date && sleep 2 && echo done`), so one approval covered them. A refusal wasn't tried. The CLI's settings are in `~/.cursor/cli-config.json` (`approvalMode`, `permissions.allow` such as `Shell(ls)`, `sandbox.mode`).

### A live run, with the real hooks (2026-10-04)

`install cursor`, then `agent -p` asking for two subagents and a `claude -p`:

- **Cursor runs Claude Code's hooks too** (from `~/.claude/settings.json`), with its own payloads: Claude Code's adapter made a `claude-code:<chat id>` session of the chat, from events it didn't know. It now ignores any payload with `cursor_version`. Until that's released, anyone with both installed gets a stray Claude Code session for each Cursor chat.
- **In `agent -p`, subagents fire no `subagentStart` or `subagentStop`**, only their `Task` calls' `preToolUse`. They show as requested, and the requests close when the run ends.
- `claude -p` was refused in `-p` mode (the shell's sandbox), so linking a session a chat starts is tested only in `tests/`.

### The stress test (2026-10-04)

Cursor's own agent ran [cursor-stress-test.md](cursor-stress-test.md) in the app, driving the CLI from its shell: about 80 chats, CLI and app, with approvals, questions, plans, subagents, resumes, interrupts and load. What held: linking (CLI chats under the app chat that started them; Claude Code, Codex and Cursor's CLI under the chats that started them), app subagents paired with their calls (three at once, Best-of-N's three), foreground subagents canceled with a stopped turn, the CLI's allowlist, questions at a turn's end, resumed chats (five ways, one session), and six runs at once. No hook failed. What it found, and what changed:

- **Commands proposed together** each fire `beforeShellExecution` at once, before Cursor asks about the first, so one finishing cleared the "may be waiting" of all. Each is now tracked on its own.
- **The CLI's `--force`, `--yolo`, `-p` and `--auto-review`** override its settings, so commands that ran without asking were "Waiting for your approval" until they ended. The CLI's command line is now read. Switching to Run Everything within a session (Shift+Tab) still isn't seen.
- **The CLI's subagents' hooks** come in conversations of their own, with nothing marking them as a subagent's (in the app they carry `parent_tool_call_id`). 36 became sessions stuck "working". Conversations that never started aren't recorded now. (Once, six `agent -p` runs started at once sent no `sessionStart` either, for no reason found; they're left out too.)
- **Plan mode** shows in `beforeSubmitPrompt`'s `composer_mode`, so a Plan-mode turn now ends on "Plan ready for your review".
- **Esc in the CLI** ends the turn with `status: error`; it's no longer shown as an error.

Cursor's, which Agent Graph can't see past: a turn started by a background subagent's result fires no `beforeSubmitPrompt`, `afterAgentResponse` or `stop`; the CLI fires no `subagentStart` or `subagentStop`; a background subagent's `subagentStop` never comes (it shows working until its chat ends); a subagent waiting for approval can't be told from its chat; quitting Cursor or archiving a chat sends no `sessionEnd`; and in the CLI, refusing a command (`n`) opens a box for what to do instead, with no hook, so it stays "Waiting for your approval", which it is, in a way. In Run Everything mode, every command is outside the sandbox, so each records two small events (most of a busy chat's log); that's accepted.

### The adapter (`src/adapter/cursor.rs`)

Node ids: `cursor:<conversation_id>` for a session, and `cursor:<conversation_id>/<subagent_id>` for a subagent. The file key is the conversation's, so a session's subagents share its events file.

| Hook | Events |
|---|---|
| `sessionStart` | `session.started` (`cwd` from `workspace_roots`, `transcript_path`) |
| `sessionEnd` | `session.ended`; also `agent.finished: canceled` for its agents still running when `reason` is aborted, window_close or user_close |
| `beforeSubmitPrompt` | `status: working`; in Plan mode (`composer_mode: plan`), also `activity` `Plan mode`, so the turn's end is "Plan ready for your review" |
| `stop` | `status: idle` (an `error` isn't shown as one: the CLI says it of a turn stopped with Esc) |
| `subagentStart` | `agent.spawned` under its parent (`parent_conversation_id`), with `subagent_type` and `task` as its purpose, cut short, and the `Task` call that started it (`tool_call_id`), so it's paired with that call exactly, not guessed |
| `subagentStop` | `agent.finished` with its `status` (the summary only when bodies are captured) |
| `preToolUse` `Task` | `spawn.requested` (kind agent, background as the call says) |
| `postToolUse` `Task` | `spawn.returned` (Cursor didn't send one in step 1) |
| `postToolUseFailure` `Task` | `spawn.returned`, with Cursor's `failure_type` |
| `postToolUse` todo tool | `tasks.updated`, if Cursor ever sends it (it didn't in step 1) |
| `preToolUse` `Shell` | `spawn.requested` (kind session) when it starts an agent, from the shell parser, as for the others |
| `postToolUse` `Shell` | `spawn.returned` for an agent launch |
| `beforeShellExecution`, outside the sandbox | `activity` with `may_ask`, labelled by a key from its turn and its text, or in the CLI's Allowlist mode, `status: input_required` for a command not on the list (see below) |
| `afterShellExecution`, outside the sandbox | `activity` with the same label, without `may_ask`: that command is done with |
| `afterAgentResponse` ending on a question | `status: input_required` with `turn_end` |
| `postToolUseFailure` `Shell` | the same, for a command that failed or that you refused |
| `preToolUse` `AskQuestion` or `CreatePlan` | `status: input_required` (Cursor doesn't send these yet: see below) |

Any hook with `parent_tool_call_id` (inside an app subagent) records nothing, and nothing is recorded for a conversation until it has started (`sessionStart`: `emit` records a hook only if it starts the conversation or the conversation's events file exists), which leaves out the CLI's subagents' conversations, whose hooks carry nothing that marks them as a subagent's. Labels are cut to 200 characters, and prompts, commands and outputs aren't kept unless body capture is on, as for the others. Every hook prints `{}`.

### Needing you, without an event for it

No hook says Cursor is waiting for you, and Agent Graph **doesn't read or follow Cursor's transcripts** for it: that would put far more in the logs than the graph needs, and much of a transcript is private. It goes only on what hooks say, which is more than Claude Code's and Codex's hooks say in some ways:

1. **Commands outside the sandbox** (for the app, settled for sure by its database: see "What an app chat is waiting on you for") (`beforeShellExecution` with `sandbox: false`). A command in the sandbox never asks, so it records nothing. One outside it records `activity` with `may_ask`, labelled by a key from its turn and its text. If it's still pending 10 seconds later (`reducer::MAY_ASK_AFTER`), the chat is shown as **"May be waiting for your approval to run a command"**: a guess, since a slow command looks the same. Each command is tracked on its own, since Cursor runs `beforeShellExecution` for every command it proposes at once, then asks about them one at a time: one ends with `afterShellExecution` (it ran) or `postToolUseFailure` (it failed, or you refused it), and all of them with the turn. The graph says when it will next change with nothing new happening (`recheck_ms` in the page's graph reply), so the page looks again then.
2. **The CLI's allowlist.** In the CLI (its hooks have `CURSOR_INVOKED_AS`), unless its command line says otherwise (`--force`, `--yolo` or `-p`, which never ask; `--auto-review`, which leaves it to a classifier, so to 1), read from the process running the hook, `~/.cursor/cli-config.json` and a project's `.cursor/cli.json` say for sure: in its Allowlist mode (`approvalMode: "allowlist"`), a command whose first word no `Shell(...)` rule names is asked about, so it's **"Waiting for your approval to run a command"** at once, and one a rule names isn't recorded at all. A chain (`&&`, `|`, `;`), which Cursor's docs don't say how it matches, falls back to 1. So do the app (whose settings are in its own database, in no documented form) and the other run modes (Auto-Review's classifier decides).
3. **A reply ending on a question.** `afterAgentResponse` has the turn's last reply. One ending on a question (past any closing formatting) makes the chat **"Asks you a question"** (the question itself only with body capture). It and `stop` come together, in either order (hooks run in parallel), so the reply's status is marked `turn_end`, and an idle landing within 2 seconds of it doesn't replace it.
4. **Plans.** A turn sent in Plan mode (`beforeSubmitPrompt`'s `composer_mode: plan`; Ask mode is `chat`, and `sessionStart` always says `agent`) ends on **"Plan ready for your review"**. A switch to Plan mode within a turn (the agent's own `SwitchMode`) fires no hook, so it isn't seen. The CLI's hooks carry no mode.
5. **Questions asked with Cursor's question tool** fire no hooks at all; in the app, its database shows them (see "What an app chat is waiting on you for"), and in the CLI there's no such tool. When Cursor fixes its bug and `AskQuestion` and `CreatePlan` reach `preToolUse`, they become "Asks: <the question>" and "Plan ready for your review": the adapter handles them already.

The FAQ and the install tab should say plainly that Cursor's approvals and questions are shown only as far as its hooks allow.

### Linking sessions

- **Started from Cursor's shell.** Cursor's commands have `CURSOR_CONVERSATION_ID`, read as `CODEX_THREAD_ID` is for Codex: a session starting with it set is the chat's child. Sessions that pass `AGENT_GRAPH_PARENT` on also pass the `CURSOR_CONVERSATION_ID` they saw (`AGENT_GRAPH_PARENT_CURSOR`), so one further down can tell which is nearer. In a subagent's commands it's the subagent's own conversation, which nothing names, so those link to nothing. (`sessionStart`'s `env` doesn't reach commands.)
- **Starting Cursor from Claude Code or Codex.** `cursor-agent` is already among the shell parser's agents. Add `agent`, but only with Cursor's flags (`-p`, `--resume`, `--model`), since `agent` alone is too common a name to count as Cursor.
- The process tree links what the environment doesn't, as now.

### Installing (`agent-graph install cursor`)

`install cursor` already exists, hidden, and installs only the `/agent-graph` skill. It will:

- write the hooks to `~/.cursor/hooks.json` (with `--scope project`, `.cursor/hooks.json`), keeping anything else there and backing it up first, as for the others. Tool hooks match only `Task` and the todo tool. Every hook is synchronous with a short timeout, since Cursor has no `async`, and `emit` takes well under 0.1 s;
- stop being hidden, and keep installing the skill;
- `uninstall cursor` removes all of it.

### Everything else that names a provider

- The adapter registry (`PROVIDERS`, `by_name`), the reducer's `PROVIDER_PROGRAMS` (`cursor` runs `cursor-agent` and `agent`), and the tree's and viewer's provider name and mark.
- **Resume:** `agent --resume <id>` in the session's folder for a CLI session; a session from the app opens back in the app, if it has a link for a chat (step 1).
- **Names:** Cursor names a chat during its first turn ("Runbook execution steps"). A CLI chat's name is the `title` in `~/.cursor/chats/<folder's hash>/<chat id>/meta.json` (a print run has none). An app chat's is in the app's global `state.vscdb` (SQLite; on macOS in `~/Library/Application Support/Cursor/User/globalStorage/`), in the `composerHeaders` table, as `name` in its JSON `value`. The hooks read it at each prompt and turn's end (`status`'s `title`; read with SQLite compiled into `agent-graph`, `rusqlite`'s bundled build, read-only, in a millisecond or so), and the viewer reads both again when they change, so a rename shows between turns. - **What the app saves beyond its hooks:** each app chat's record in that database also has its **todo list** (`todos` in its `composerData:<id>` row in `cursorDiskKV`: `[{"id", "content", "status"}]`) and **`hasPendingPlan`** (in `composerHeaders`). Watched live (2026-10-04): Cursor saves a chat's record when it gets round to it, not as things happen. The todo list is saved within a second of each change, so the `stop` hook records it as the chat's tasks (`tasks.updated`), and the viewer shows it live. A plan's being ready is saved about 80 ms after the turn's `stop` hook, too late for the hook, so only the viewer uses it: an idle chat with a plan waiting is "Plan ready for your review", which also catches a plan reached by the agent switching itself into Plan mode, which no hook says. The viewer reads both for the 20 most recently active app chats, whenever the database changes. `hasBlockingPendingActions` is true while a command waits for approval, but an approval prompt doesn't make Cursor save, so it's often stale, and isn't used: approvals stay with the hooks. A chat's `status` reads `aborted` while a turn runs and `completed` once it's over.
- **What an app chat is waiting on you for:** Cursor saves each tool call's message as it's made (`bubbleId:<chat>:<message>` in `cursorDiskKV`, with `toolFormerData`), and saves it again when it's answered. Watched live (2026-10-04), with the earlier version recovered from the `-wal` file: a command waiting for approval is saved, as Cursor asks, with `additionalData.reviewData.status: "Requested"` (then `Done`, with `selectedOption` `run` or `skip`); a question from the question tool, as it's shown, as an `ask_question` that's `loading` with no `userDecision` (then `completed`, `accepted`). A plan's `create_plan` keeps `Requested` once it's built, so it isn't counted (`hasPendingPlan` is), and a tool call that ran without asking has no `reviewData`. So for app chats there's no guessing: the viewer shows "Waiting for your approval" or "Asks you a question" for a chat at work whose database says so, and where its database says nothing's waiting, it drops the hooks' guess (a slow command isn't shown as waiting). The viewer and `watch-remote` also write these into the chat's log as they start and stop (a `status` from `cursor-app@1`), for chats active in the last half hour, so the site shows them; the site still makes the hooks' guess for a slow command, which it can't check. The CLI saves nothing while it waits (its record changed only before it asked, and after), so its allowlist and the guess are all there is.
- **Archived chats:** Cursor runs no hook when an app chat is archived (and never sends `sessionEnd` for one), so an archived chat would stay idle for ever. The app's database marks it (`composerHeaders.isArchived`), so the viewer and `watch-remote`, every few seconds, append `session.ended` (reason `archived`) to an archived chat's log that hasn't ended, as they mark the Claude app's blocked sessions: the site shows it too.
- **The CLI's own database:** each CLI chat has one, `store.db` (SQLite) beside its `meta.json`. Its `meta` table holds a hex-encoded JSON object: `name`, `mode` (`plan` in Plan mode, else `default`), `isRunEverything`, `approvalMode` (`auto-review`, or `unrestricted` with Run Everything; not there for the settings' allowlist), and, in a subagent's own conversation, `subagentInfo` (`parentAgentId`, `rootParentAgentId`, `toolCallId`, `typeName`). Its `blobs` are content-addressed by SHA-256: the root (`latestRootBlobId`, a protobuf message) names the todo records in its field 3, each one `{1: id, 2: text, 3: status}` (1 pending, 2 in progress, 3 completed, 4 cancelled). The messages are blobs too, and aren't read. Agent Graph uses it:
  - **CLI subagents:** a hook in a subagent's conversation is routed by its `subagentInfo` to the subagent (`cursor:<chat>/<its Task call>`), in its chat's events file. It's announced, with its call, at each of its tool calls. Its commands, and their approvals, are its own. The CLI never says a subagent has finished, so a foreground one still open when its chat's turn ends finished with it, unless the turn was stopped (`stop` with `aborted`, marked by `activity` `Turn stopped`), when it's canceled.
  - **Plans:** a turn in a chat whose `mode` is `plan` ends on "Plan ready for your review".
  - **Todos:** read at each turn's end (`stop`, or a print run's `sessionEnd` with `completed`, which is its turn's end too), before the turn's status, and by the viewer when the chat's database changes.
  - **Run mode:** `isRunEverything` (chosen at an approval prompt, with Shift+Tab: the chat's record says so at once) means it never asks, and `auto-review` leaves it to a classifier. A command's end is judged as its start was, by how the CLI was started, since Run Everything may have been chosen in answer to it.
- **"This session"** for the `/agent-graph` skill: `AGENT_GRAPH_PARENT` from `sessionStart`'s `env`, or the `CURSOR_*` variable if there is one.
- The README, `docs/design.md` §5.4 (which still says it's unknown whether the CLI fires hooks: it does), `docs/cli-help.json`, the site's install tabs (a "Cursor" tab), its FAQ, and the home page, which says nothing of Cursor until this ships. Rebuild the site's WebAssembly, since the reducer changes.

### Testing

- **Unit:** the adapter, with step 1's payloads (`tests/fixtures/cursor/`), through the reducer (`tests/cursor.rs`). That covers subagents with and without `subagentStop`, a command pending past the threshold, and `{}` on stdout for every event. Until step 1, the fixtures are written from Cursor's docs, and say so. Installing and uninstalling are tested in `src/install.rs`.
- **End to end:** the CLI can't run against a stand-in model, as Codex's can, since it only talks to Cursor's servers. So `scripts/cursor-e2e.sh` runs the real `agent -p` with a temporary `HOME`/`AGENT_GRAPH_HOME` and your Cursor login (`CURSOR_API_KEY`), on prompts that make a todo list and a subagent, and checks the graph. It's run by hand, not in CI, since it costs a little of your Cursor usage.
- **The app, by hand:** step 1's prompts again with the real hooks installed, watched in `agent-graph view`.
- **Windows:** the hook command through PowerShell, as Codex's is tested on Windows CI, with `{}` on stdout.

## Cloud

**Not built yet** (2026-10-04). Nothing here has been tried: it starts with step 1, a probe in a real cloud agent.

Cursor's cloud agents run in a VM per agent (Linux), with the repository checked out. They're set up by `.cursor/environment.json` (`install`, `start`, `terminals`, a Dockerfile or a snapshot), with secrets from Cursor's dashboard as environment variables. They're started from cursor.com/agents, the app, the phone app, Slack, GitHub, Linear or the API, and a cloud agent's subagents run on VMs of their own (Cursor's docs).

### What's changed since this was first planned

The local work, and its tests, settled things that change the cloud plan:

- **`emit` drops a conversation that hasn't started.** Locally, `emit` records a Cursor conversation only once its `sessionStart` has been seen (its events file exists), which keeps the CLI's subagents' conversations out. Cloud agents don't run `sessionStart`, by Cursor's docs, so as things are, **every cloud hook would be dropped**. In the cloud, the first hook must start the session itself (`session.started`, then what it's for), and a cloud subagent's conversation must be told apart from a cloud agent's in some other way (step 1 shows what its hooks carry).
- **Much of the local picture comes from Cursor's own databases, on the computer**: chats' names, todo lists, a plan waiting, and what an app chat is waiting on you for (the app's `state.vscdb`); and, for the CLI, the same in each chat's `~/.cursor/chats/…/store.db`, with which conversation a subagent's is (`subagentInfo`). In a cloud VM, there's no app's database, and maybe no CLI's: step 1 looks. Without them, a cloud agent is recorded from its hooks alone: what they carry, with the 10-second guess for commands outside the sandbox, and no names, todo lists or questions. (If a cloud agent's VM has `~/.cursor/chats`, the CLI's reading applies there as it does here.)
- **A hook must print `{}`**, and the hook command must work in the VM's shell. `emit` already prints `{}`, and the Linux build (both architectures, SQLite compiled in, needing nothing installed) has been run on a Linux VM with no `sqlite3`.
- **Cloud subagents run on VMs of their own.** Their hooks run in their VM, from the same repository's hooks file, so they'd record and share themselves: as separate sessions unless their hooks name their parent (`parent_conversation_id`, or `subagentInfo` in a VM's CLI store). Step 1 starts one to see.
- **`agent worker`** (Cursor's CLI: "a self-hosted Cloud Agent worker … on this machine", My Machines, with no Enterprise plan needed) runs cloud agents on a computer of your own. Those would run their hooks there: your own `~/.cursor/hooks.json`, so recorded and shared as local chats already are, if their hooks are the CLI's. A third way in (C, below), to try after A.

### A. Hooks in the repository (as Claude Code's cloud)

The docs say cloud agents run **command hooks from the repository's `.cursor/hooks.json`**, but not `sessionStart`, `sessionEnd` or the MCP hooks. Everything else fires. That's Claude Code's cloud again, where the hooks are committed to the repository and do nothing anywhere else:

- `agent-graph install cursor --cloud` adds our hooks to the project's `.cursor/hooks.json`, to be committed. Each starts with a guard, so it does nothing but in Cursor's cloud: `CURSOR_CODE_REMOTE` (which Cursor's docs list among hooks' variables), if step 1 shows it's set there, else whatever is. So a teammate's local Cursor doesn't record twice, or at all if they haven't installed Agent Graph.
- **The first hook to fire** in a conversation (`beforeSubmitPrompt`, or whatever step 1 shows comes first):
  - installs `agent-graph` if it isn't there (the site's `install.sh`, from the VM, as Claude Code's cloud hook does; or `.cursor/environment.json`'s `install`, which is cached in the snapshot);
  - records `session.started` itself, so `emit`'s start check is met (a cloud mode of `emit`: `--cloud`, or the guard's variable, says a hook may start a session);
  - starts `watch-remote --background`, once (a pid file says whether it's running; each hook checks, so a VM paused and resumed between turns starts it again).
- **The token:** `AGENT_GRAPH_TOKEN` as a secret in Cursor's dashboard (Cloud Agents → Secrets), which reaches the VM as an environment variable. The model can read it, as in Codex's cloud. It only shares to your account, and can be deleted from your account page.
- **The host label:** "Cursor cloud" (`account::host_name`, beside "Claude Code cloud" and "Codex cloud").
- **Covers** every cloud agent on that repository, wherever it was started.
- **What it can't see**, beyond what the hooks don't say: a cloud agent's approvals (if it asks for any: cloud agents mostly run without), its questions and plans (no hooks, and no app's database), and its end (no `sessionEnd`: a cloud agent's turn ends with `stop`, and an agent that's done stays idle until the share goes quiet).

**Step 1 for A (you, about 15 minutes).** This is at cursor.com/agents, on a throwaway branch of a repository Cursor's cloud agents can reach (a private throwaway repository on GitHub is fine: `gh repo create --private` can make one):

1. On your computer, in that repository, run `sh <this repository>/scripts/cursor-probe.sh install --project`, and commit the `.cursor/hooks.json` and `scripts/cursor-probe.sh` it writes to that branch. Push it.
2. In Cursor's dashboard (Cloud Agents → Secrets), add `AGENT_GRAPH_PROBE` with any made-up value.
3. At cursor.com/agents, start an agent on that branch with:
   > Run each of these shell commands, and show me all their output, without changing anything: `env | cut -d= -f1 | sort`; `echo CURSOR_CODE_REMOTE=$CURSOR_CODE_REMOTE AGENT_GRAPH_PROBE=${AGENT_GRAPH_PROBE:+set}`; `id; uname -a; echo HOME=$HOME`; `ps -eo pid,ppid,args | head -40`; `ls -la ~/.cursor ~/.cursor/chats 2>&1 | head -40`; `cat ~/.agent-graph-probe/cursor.jsonl | cut -c1-400`; `curl -sS -o /dev/null -w '%{http_code}\n' https://agentgraph.chofter.com/install.sh`.
4. Send it a follow-up:
   > Now use one subagent to count the files in the repository and report the count. Then show me the output of `cat ~/.agent-graph-probe/cursor-env.txt` and `cat ~/.agent-graph-probe/cursor.jsonl | cut -c1-400`.
5. Paste me both answers.

That shows:

- whether hooks run in the cloud, which (`sessionStart`?), and in what order, so which hook comes first;
- what marks the cloud (`CURSOR_CODE_REMOTE`, `is_background_agent`, or another variable), for the guard;
- whether the secret reaches the agent's commands and hooks, and whether the VM can reach agentgraph.chofter.com;
- whether there's a CLI-style `~/.cursor/chats` in the VM (and so names and todo lists there), and what process runs the agent;
- what a cloud subagent's hooks carry, and whether they run in the parent's VM or one of their own (the second answer shows whether the subagent's hooks reached the first VM's log);
- whether the VM, and a background process in it, lasts between turns (the probe's log, from the first turn, still there in the second).

### B. Cursor's cloud agents API (if A falls short)

Cursor's API (`api.cursor.com`, an API key from cursor.com/dashboard) lists every cloud agent (`GET /v1/agents`) and its runs, and streams a run as it goes (`GET /v1/agents/{id}/runs/{runId}/stream`). The stream has status, assistant text, `tool_call` started and completed, and turn ended. That covers agents on repositories with nothing committed, and needs nothing in the VM:

- `agent-graph watch-remote --cursor-cloud` (a mode of `watch-remote`, with `CURSOR_API_KEY`) polls the agent list, follows each active run's stream, turns them into the same events (session per agent, `bc-…`; run status to working, idle or failed; `Task` calls to subagents, as far as the stream shows them), and shares them;
- **its catch:** it runs on your computer, so it only records while that's on. Running it from the site instead would mean the site keeping your Cursor API key and polling for you. That's a bigger step, with its own security review, so not now;
- **what it can't see:** questions and plan approvals, since the API has no "waiting for you" status. A run that ends on a question shows as idle. As locally, the stream's text isn't kept or read for this.

Build B only if A's step 1 shows cloud hooks don't run, or users ask for agents on repositories they can't commit to.

### C. Cloud agents on your own computer (`agent worker`)

`agent worker` runs Cursor's cloud agents on a computer of your own (My Machines). If their hooks are your own `~/.cursor/hooks.json`, as the CLI's are, they're recorded, named, and shared like any local chat, with the CLI's database, and nothing to commit. To try after A: start a worker on this Mac, send it a cloud agent, and see what its hooks are and carry.

### The site

- A "Cursor cloud" tab in the install section, beside "Claude Code cloud" and "Codex cloud" (`/#install-cursor-cloud`). It says where each step happens: the command to run in the repository on your computer, what to commit, and where the secret goes in Cursor's dashboard.
- The account page's new-token note links to it too.

## Order of work

1. ~~Local step 1, then the adapter, installing and the needs-you checks, with tests.~~ Done (2026-10-04), with what Cursor's own databases add: names, todo lists, plans, what an app chat's waiting on you for, CLI subagents, archived chats. Still to do locally: resuming a CLI chat (`agent --resume <id>`), un-hiding `install cursor`, and the site's "Cursor" tab and FAQ. Then a release, which also ends the stray Claude Code session that Homebrew's 0.1.7 makes of every Cursor chat.
2. Cloud step 1 (you), then plan A: `install cursor --cloud`, a cloud mode for `emit` (the first hook starts the session), the host label, and "Cursor cloud" on the site.
3. C, to see whether cloud agents on your own computer come for free.
4. B, only if needed.

## Risks

- **"Needs you" is partial.** Until Cursor fixes its hooks, questions asked with its question tool and plans aren't shown, and command approvals in the app are a guess from a timer. The viewer's wording should never claim more than we know.
- **Subagent hooks break between versions.** `postToolUse` for `Task` backs up `subagentStop`, and `stop` and `sessionEnd` end any agent left running. So a missed hook shows an agent finishing late, never one running for ever.
- **Cursor moves fast.** Hook names and payloads have changed between 1.7, 2.x and 3.x. Fixtures should note the version they're from, and the adapter should take unknown fields and events without failing.
- **The cloud token is readable by the model**, as in Codex's cloud (see A).
