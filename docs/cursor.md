# Cursor support: plan

**Built so far (not yet released):** the local adapter (`src/adapter/cursor.rs`), checked against a real Cursor (step 1, below), with tests on its payloads (`tests/cursor.rs`). `emit` answers Cursor's hooks with `{}`. Sessions a chat starts link to it through `CURSOR_CONVERSATION_ID`. A subagent is paired with the exact `Task` call that started it (`agent.spawned`'s `call_id`), and one still at work when its turn is stopped is canceled. `install cursor` writes the hooks, but stays hidden until it has been run for real. Not built yet: resuming a chat, the site, and the cloud.

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

### A live run, with the real hooks (2026-10-04)

`install cursor`, then `agent -p` asking for two subagents and a `claude -p`:

- **Cursor runs Claude Code's hooks too** (from `~/.claude/settings.json`), with its own payloads: Claude Code's adapter made a `claude-code:<chat id>` session of the chat, from events it didn't know. It now ignores any payload with `cursor_version`. Until that's released, anyone with both installed gets a stray Claude Code session for each Cursor chat.
- **In `agent -p`, subagents fire no `subagentStart` or `subagentStop`**, only their `Task` calls' `preToolUse`. They show as requested, and the requests close when the run ends.
- `claude -p` was refused in `-p` mode (the shell's sandbox), so linking a session a chat starts is tested only in `tests/`.

### The adapter (`src/adapter/cursor.rs`)

Node ids: `cursor:<conversation_id>` for a session, and `cursor:<conversation_id>/<subagent_id>` for a subagent. The file key is the conversation's, so a session's subagents share its events file.

| Hook | Events |
|---|---|
| `sessionStart` | `session.started` (`cwd` from `workspace_roots`, `transcript_path`) |
| `sessionEnd` | `session.ended`; also `agent.finished: canceled` for its agents still running when `reason` is aborted, window_close or user_close |
| `beforeSubmitPrompt` | `status: working` |
| `stop` | `status: idle`; `agent.finished: failed` with Cursor's error when `status` is error |
| `subagentStart` | `agent.spawned` under its parent (`parent_conversation_id`), with `subagent_type` and `task` as its purpose, cut short, and the `Task` call that started it (`tool_call_id`), so it's paired with that call exactly, not guessed |
| `subagentStop` | `agent.finished` with its `status` (the summary only when bodies are captured) |
| `preToolUse` `Task` | `spawn.requested` (kind agent, background as the call says) |
| `postToolUse` `Task` | `spawn.returned` (Cursor didn't send one in step 1) |
| `postToolUseFailure` `Task` | `agent.finished: failed` |
| `postToolUse` todo tool | `tasks.updated`, if Cursor ever sends it (it didn't in step 1) |
| `preToolUse` `Shell` | `spawn.requested` (kind session) when it starts an agent, from the shell parser, as for the others |
| `postToolUse` `Shell` | `spawn.returned` for an agent launch |
| `preToolUse` `AskQuestion` or `CreatePlan` | `status: input_required` (Cursor doesn't send these yet: see below) |

Any hook with `parent_tool_call_id` (inside a subagent) records nothing. Labels are cut to 200 characters, and prompts, commands and outputs aren't kept unless body capture is on, as for the others. Every hook prints `{}`.

### Needing you, without an event for it

This is the one real gap, and it matters most, since "needs you" is what Agent Graph is for. Agent Graph **doesn't read or follow Cursor's transcripts** for it: that would put far more in the logs than the graph needs, and much of a transcript is private (your code, prompts and the model's output). It goes only on what hooks say. That means:

1. **Commands waiting for approval.** (Step 1 saw none: Cursor didn't ask. Not built until a run with approvals on shows what fires.) If it shows `beforeShellExecution` fires before Cursor asks you, and `afterShellExecution` only once the command has run, then a command started and not finished is shown as "Running a command". After about 10 seconds that becomes "Running a command (it may need your approval)". The wording says it's a guess, since a slow command looks the same. It needs a clock in the reducer, as "went quiet" already has. If step 1 shows Cursor asks before `beforeShellExecution` fires, there's nothing to go on, and approvals aren't shown.
2. **Questions and plans** fire no hooks at all, so they aren't shown. The session stays "working" until you answer and something fires again. When Cursor fixes its bug and `AskQuestion` and `CreatePlan` reach `preToolUse`, they become "Asks: <the question>" and "Plan ready for your review", as for Claude Code and Codex. The adapter handles those tool names already, so it'll work as soon as Cursor sends them.
3. **The turn ending** (`stop`) is reliable, and shows the session as idle, which the viewer already counts as your turn.

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
- **"This session"** for the `/agent-graph` skill: `AGENT_GRAPH_PARENT` from `sessionStart`'s `env`, or the `CURSOR_*` variable if there is one.
- The README, `docs/design.md` §5.4 (which still says it's unknown whether the CLI fires hooks: it does), `docs/cli-help.json`, the site's install tabs (a "Cursor" tab), its FAQ, and the home page, which says nothing of Cursor until this ships. Rebuild the site's WebAssembly, since the reducer changes.

### Testing

- **Unit:** the adapter, with step 1's payloads (`tests/fixtures/cursor/`), through the reducer (`tests/cursor.rs`). That covers subagents with and without `subagentStop`, a command pending past the threshold, and `{}` on stdout for every event. Until step 1, the fixtures are written from Cursor's docs, and say so. Installing and uninstalling are tested in `src/install.rs`.
- **End to end:** the CLI can't run against a stand-in model, as Codex's can, since it only talks to Cursor's servers. So `scripts/cursor-e2e.sh` runs the real `agent -p` with a temporary `HOME`/`AGENT_GRAPH_HOME` and your Cursor login (`CURSOR_API_KEY`), on prompts that make a todo list and a subagent, and checks the graph. It's run by hand, not in CI, since it costs a little of your Cursor usage.
- **The app, by hand:** step 1's prompts again with the real hooks installed, watched in `agent-graph view`.
- **Windows:** the hook command through PowerShell, as Codex's is tested on Windows CI, with `{}` on stdout.

## Cloud

Cursor's cloud agents run in a VM per agent, with the repository checked out. They're set up by `.cursor/environment.json` (`install`, `start`, `terminals`, a Dockerfile or a snapshot), with secrets from Cursor's dashboard as environment variables.

Two ways in. They aren't exclusive, and A comes first.

### A. Hooks in the repository (as Claude Code's cloud)

The docs say cloud agents run **command hooks from the repository's `.cursor/hooks.json`**, but not `sessionStart`, `sessionEnd` or the MCP hooks. Everything else fires. That's Claude Code's cloud again, where the hooks are committed to the repository and do nothing anywhere else:

- `agent-graph install cursor --cloud` adds our hooks to the project's `.cursor/hooks.json`, to be committed. Each starts with a guard, so it does nothing but in Cursor's cloud. That's `CURSOR_CODE_REMOTE`, if step 1 shows it's set there. Otherwise it's whatever is. So a teammate's local Cursor doesn't record twice, or record at all if they haven't installed Agent Graph.
- With no `sessionStart`, the **first hook to fire** in a conversation (`beforeSubmitPrompt`) does what Claude Code cloud's `SessionStart` hooks do:
  - installs `agent-graph` if it isn't there (or `.cursor/environment.json`'s `install` does, which is cached in the snapshot);
  - records `session.started`;
  - starts `watch-remote --background`, once.
- **The token:** `AGENT_GRAPH_TOKEN` as a secret in Cursor's dashboard (Cloud Agents → Secrets), which reaches the VM as an environment variable. The model can read it, as in Codex's cloud. It only shares to your account, and can be deleted from your account page.
- **The host label:** "Cursor cloud" (`account::host_name`, beside "Claude Code cloud" and "Codex cloud").
- **Covers** every cloud agent on that repository, wherever it was started: the web, the app, the phone app, Slack, GitHub or Linear.

**Step 1 for A (you, about 15 minutes).** This is at cursor.com/agents, on a throwaway branch of a repository Cursor's cloud agents can reach:

1. On your computer, in that repository, run `sh <this repository>/scripts/cursor-probe.sh install --project`, and commit the `.cursor/hooks.json` and `scripts/cursor-probe.sh` it writes to that branch.
2. In Cursor's dashboard (Cloud Agents → Secrets), add `AGENT_GRAPH_PROBE` with any made-up value.
3. At cursor.com/agents, start an agent on that branch with: "Run `env | cut -d= -f1 | sort`, `echo CURSOR_CODE_REMOTE=$CURSOR_CODE_REMOTE`, `id`, `ps aux | head -30`, `cat ~/.agent-graph-probe/cursor.jsonl`, and `curl -sS -o /dev/null -w '%{http_code}\n' https://agentgraph.chofter.com/install.sh`, and show me all their output without changing anything." Then send it one follow-up, "Now use a subagent to count the files in the repository, then show me `cat ~/.agent-graph-probe/cursor-env.txt`", and paste me both answers.

That shows whether hooks run and which, what marks the cloud (`CURSOR_CODE_REMOTE` or another variable), whether secrets reach the agent, whether the agent can reach agentgraph.chofter.com, and whether a background process (`watch-remote`) lasts between a VM's turns. If the VM is paused between follow-ups, `watch-remote` must be started again by the next turn's first hook. Each hook checks for it cheaply, from a pid file.

### B. Cursor's cloud agents API (later, if A falls short)

Cursor's API (`api.cursor.com`, an API key from cursor.com/dashboard) lists every cloud agent (`GET /v1/agents`) and its runs, and streams a run as it goes (`GET /v1/agents/{id}/runs/{runId}/stream`). The stream has status, assistant text, `tool_call` started and completed, and turn ended. That covers agents on repositories with nothing committed, and needs nothing in the VM:

- `agent-graph watch-remote --cursor-cloud` (a mode of `watch-remote`, with `CURSOR_API_KEY`) polls the agent list, follows each active run's stream, turns them into the same events (session per agent, `bc-…`; run status to working, idle or failed; `Task` calls to subagents, as far as the stream shows them), and shares them;
- **its catch:** it runs on your computer, so it only records while that's on. Running it from the site instead would mean the site keeping your Cursor API key and polling for you. That's a bigger step, with its own security review, so not now;
- **what it can't see:** questions and plan approvals, since the API has no "waiting for you" status. A run that ends on a question shows as idle. As locally, the stream's text isn't kept or read for this.

Build B only if A's step 1 shows cloud hooks don't run, or users ask for agents on repositories they can't commit to.

### The site

- A "Cursor cloud" tab in the install section, beside "Claude Code cloud" and "Codex cloud" (`/#install-cursor-cloud`). It says where each step happens: the command to run in the repository on your computer, what to commit, and where the secret goes in Cursor's dashboard.
- The account page's new-token note links to it too.

## Order of work

1. Local step 1 (you), then the adapter, installing and the needs-you checks, with tests. Ship as "Cursor" (the app and CLI), with the site's tab and FAQ.
2. Cloud step 1 (you), then plan A, with "Cursor cloud" on the site.
3. B, only if needed.

## Risks

- **"Needs you" is partial.** Until Cursor fixes its hooks, questions and plans aren't shown, and command approvals are at best a guess from a timer. The viewer's wording should never claim more than we know.
- **Subagent hooks break between versions.** `postToolUse` for `Task` backs up `subagentStop`, and `stop` and `sessionEnd` end any agent left running. So a missed hook shows an agent finishing late, never one running for ever.
- **Cursor moves fast.** Hook names and payloads have changed between 1.7, 2.x and 3.x. Fixtures should note the version they're from, and the adapter should take unknown fields and events without failing.
- **The cloud token is readable by the model**, as in Codex's cloud (see A).
