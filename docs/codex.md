# Codex support: plan

Agent Graph records Claude Code's sessions through its hooks. This is the plan for recording the Codex CLI's sessions the same way: the TUI (`codex`) and headless runs (`codex exec`), their subagents, their plans, when they're waiting for you, and the sessions they start from their shells. It was written from Codex's source (openai/codex at `60947e2`, 2026-09-30) and checked against a real Codex CLI 0.159.2 run against a stand-in model (see Testing).

## What Codex gives us

Codex's hooks are close to Claude Code's, by design: the same JSON shape in the settings, the same event names, and stdin JSON with snake_case fields.

- **Where.** `~/.codex/hooks.json` (or `$CODEX_HOME/hooks.json`), or a project's `.codex/hooks.json`. The same `{"hooks": {"Event": [{"matcher", "hooks": [{"type": "command", "command", "timeout", "async"}]}]}}` as Claude Code's `settings.json`. Hooks are on by default (`features.hooks`).
- **Trust.** Unlike Claude Code, a hook doesn't run until it's trusted. The TUI asks at start-up ("Hooks need review"); `codex exec` silently skips untrusted hooks. Trust is kept in the user's `config.toml`, one entry per handler: `[hooks.state."<file>:<event>:<group>:<handler>"] trusted_hash = "sha256:…"`, where the hash is over the handler as Codex normalizes it. If the handler changes, it's "modified" and stops running until trusted again. A project's hooks also need the project to be trusted.
- **Events.** SessionStart, SessionEnd, UserPromptSubmit, PreToolUse, PermissionRequest, PostToolUse, PreCompact, PostCompact, SubagentStart, SubagentStop, Stop, Interrupt. There's no Notification event.
  - Every payload has `session_id` (the **root** thread's id, even inside a subagent), `transcript_path` (the thread's rollout file), `cwd` and `hook_event_name`. Turn events add `turn_id`, and, inside a subagent, `agent_id` (the subagent's own thread id) and `agent_type` (its role).
  - SessionStart fires lazily, at the start of the first turn, then UserPromptSubmit. Stop fires at the end of each root turn; Interrupt when a turn is cancelled. SessionStart, SessionEnd, Stop and Interrupt never fire in a subagent: SubagentStart and SubagentStop do instead.
  - SubagentStart fires in the child; its `transcript_path` is the child's rollout, whose first line (`session_meta`) names the parent thread (`source.subagent.thread_spawn.parent_thread_id`), for nested agents.
- **Output.** Exit 0 with nothing on stdout is fine for every event. Plain stdout from SessionStart, SubagentStart or UserPromptSubmit goes into the model's context, so `emit` must stay silent, as it is. `async: true` works on every event but SessionEnd (always synchronous, with a 1 s default and 3 s cap).
- **Tools**, as hooks name them:
  - the shell (`exec_command`) is `Bash`, with `tool_input: {"command": "<string>"}`. A command still running when the call returns gets its PostToolUse later, when a poll sees it finish;
  - `apply_patch`;
  - `update_plan` (`{"plan": [{"step", "status": "pending|in_progress|completed"}]}`), which is off unless `[tools.update_plan] enabled = true`;
  - `spawn_agent` (multi-agent v1, on by default), which returns at once with `{"agent_id", "nickname"}` as a JSON string: subagents always run in the background. Then `wait_agent` (`{"targets": [ids]}`), `send_input`, `resume_agent`, `close_agent`, possibly prefixed with their namespace (`multi_agent_v1…`);
  - `request_user_input` (`{"questions": [{"question", …}]}`), in Plan mode;
  - PermissionRequest (no `tool_use_id`): `tool_name` and `tool_input` with an optional `description`. It means approval is needed, from you or Codex's auto-reviewer.
- **Environment.** Hooks get Codex's own environment, with nothing added. The shell commands the model runs get `CODEX_THREAD_ID` (the thread running them: a subagent's own id inside one) and `CODEX_SESSION_ID` (the root's). There's no equivalent of Claude Code's `CLAUDE_ENV_FILE`.
- **Names.** The TUI names a thread after its first message, and `/rename` renames it, in `$CODEX_HOME/session_index.jsonl` (append-only, `{"id", "thread_name", "updated_at"}`, the last line for an id wins). `codex exec` doesn't name threads.
- **Resuming.** `codex resume <id>`, and `codex fork <id>` for a copy.
- **Skills.** `~/.agents/skills` (where `install codex` already puts the `$agent-graph` skill).

## Design

### The adapter (`src/adapter/codex.rs`)

Node ids: `codex:<session_id>` for a session, and `codex:<session_id>/<agent_id>` for a subagent. The file key is the session's, so a session's subagents share its events file, as with Claude Code.

| Hook | Events |
|---|---|
| SessionStart | `session.started` (`cwd`, `source`, `transcript_path`, `title` from `session_index.jsonl`) |
| SessionEnd | `session.ended` |
| UserPromptSubmit | `status: working` for the node it's in (session, or subagent given more work), with the session's name |
| Stop | `status: idle`, with the session's name (the TUI names it during the first turn) |
| Interrupt | `status: idle` |
| SubagentStart | `agent.spawned` (`agent_type`), under its parent: the session, or, for a nested agent, the agent its rollout names |
| SubagentStop | `agent.finished: completed` (with its last message as the summary only when bodies are captured) |
| PermissionRequest | `status: input_required`, "Needs approval: <Codex's description of it>", or "Needs approval to run a command" (never the command itself) |
| PreToolUse `request_user_input` | `status: input_required`, "Asks: <first question>" |
| PreToolUse `Bash` that starts an agent | `spawn.requested` (kind session), as for Claude Code, from the same shell parser |
| PreToolUse `spawn_agent` | `spawn.requested` (kind agent, background: `spawn_agent` never blocks) |
| PreToolUse `wait_agent` | `wait.started` on each target, so the session shows as waiting on its agents |
| PostToolUse `spawn_agent` | `spawn.returned`, naming the child from `agent_id` |
| PostToolUse `wait_agent` | `wait.ended` for each target |
| PostToolUse `update_plan` | `tasks.updated`, the plan's steps |
| PostToolUse `Bash`, `apply_patch` | `spawn.returned` for an agent launch; otherwise, only when approvals are on (`permission_mode: default`), `status: working`, since an approval may have been answered and Codex has no event for that |
| PostToolUse `request_user_input` | `status: working` |

Labels are cut to 200 characters, and prompts, commands and outputs aren't kept, as for Claude Code. A spawned agent's purpose comes from nothing Codex sends but its prompt, so it's only kept (cut short) with body capture on; its role and nickname name it otherwise.

### Linking sessions

- **Started from Codex's shell.** Codex has no env file for passing `AGENT_GRAPH_PARENT` on, but its shell commands carry `CODEX_THREAD_ID` and `CODEX_SESSION_ID`, which name the Codex thread running them. A session starting with those set is Codex's child: `codex:<session>`, or `codex:<session>/<thread>` in a subagent. That holds whatever started Codex.
- **Which is nearer.** A session can see both `AGENT_GRAPH_PARENT` (from a Claude Code session further up) and `CODEX_THREAD_ID`. Claude Code sessions will also export the `CODEX_THREAD_ID` they saw (`AGENT_GRAPH_PARENT_CODEX`), so a child can tell: if `CODEX_THREAD_ID` is the one the exporting session saw, the export is nearer; otherwise Codex is.
- **Starting Claude Code (or Codex) from Codex** then works both ways: Codex's `Bash` PreToolUse records the request, and the child links itself back.
- The process tree still links what the environment doesn't.

### Installing (`agent-graph install codex`)

- Writes the hooks to `~/.codex/hooks.json` (`--scope project`: `.codex/hooks.json`), keeping anything else there, and backing the file up first, as for Claude Code. Tool hooks only match what matters: `Bash|apply_patch|spawn_agent|wait_agent|multi_agent_v1wait_agent|update_plan|request_user_input` (a plain list is matched exactly, and multi-agent v1's `wait_agent` reaches hooks with its namespace run into its name). SessionStart is synchronous (10 s), SessionEnd is synchronous as Codex requires (with a 3 s timeout), and the rest are in the background.
- **Trusts them**, as `/hooks` would: writes each handler's `trusted_hash` to `~/.codex/config.toml` (`$CODEX_HOME`), editing it in place so its comments and layout stay. Uninstalling removes them. Without this, `codex exec` would skip them silently, and the TUI would ask. The summary says it's doing this, and `--dry-run` shows it.
- A project's hooks also need the project trusted in Codex; the installer says so.
- Keeps adding the `$agent-graph` skill.
- `uninstall codex` removes all of it.

### Everything else that names Claude Code

- The adapter registry (`codex` alongside `claude-code`), the reducer's provider programs (`codex` runs `codex`), and the tree's provider name.
- **Resume:** `codex resume <id>` in the session's folder, or `codex fork <id>` for one still running. `codex` is found on `PATH`, or else inside the ChatGPT app, which has its own copy and doesn't put it on `PATH`.
- **The ChatGPT desktop app** runs Codex too (its bundled `codex app-server`, reading the same `~/.codex`), so the same hooks record its sessions, with nothing more to set up. A session the app started (its rollout's `originator` is "Codex Desktop") opens back in the app (`codex://threads/<id>`), as a Claude app session opens in the Claude app; the CLI's open in a terminal. The viewer also reads names Codex gives sessions (`session_index.jsonl`) as they appear, since the app names a session after its first turn has ended.
- **Plans:** Codex's plan tool is off by default, so `install codex` turns it on (`[tools.update_plan] enabled = true`, marked as Agent Graph's), unless your settings already say either way; uninstalling turns it off again only if Agent Graph turned it on.
- **"This session"** for the `$agent-graph` skill: the session whose shell it runs in, found as a child session would find its parent (so `CODEX_THREAD_ID`/`CODEX_SESSION_ID` in Codex), then `CLAUDE_CODE_SESSION_ID`.
- **Names:** a subagent is shown by its Codex nickname ("explorer Noether"; just "Peirce" for Codex's "default" type, which says nothing), from its rollout, where Claude Code's show their id.
- **Multi-agent v2**, which the ChatGPT app uses: its tools reach hooks as `collaborationspawn_agent` and so on (matched, and read without the namespace). A spawn names a task (`wait_60_seconds`), shown as the agent's purpose in words ("Wait 60 seconds"), as the app shows it; it returns no thread id, so the request is left open and the child is bound to it when it starts. An agent given more work later (`followup_task`, `send_message`; v1's `send_input`, `resume_agent`) fires no hook of its own, so the parent's call marks it working again, finding a v2 agent's thread from its task's path in the session's rollouts.
- Messages that say "run `agent-graph install claude-code`" mention Codex too. The viewer already has Codex's name and mark.
- The README, `docs/design.md` (§5.4) and `docs/cli-help.json`; the site's FAQ ("Codex is coming") and the install tabs. The site's WebAssembly is rebuilt, since the reducer changes.

### Not now

- **Codex cloud** (chatgpt.com/codex): nothing in Codex's source says whether it runs a project's hooks, or trusts them, and there's no variable that marks a cloud run. Planned in [codex-cloud.md](codex-cloud.md).
- **Multi-agent v2's waits:** its `wait_agent` names no agents, so no wait is shown.
- **The IDE extension** runs the same core, so it should fire the same hooks; untested. (The ChatGPT desktop app is tested: see Testing.)
- Windows.

## Testing

- **Unit:** the adapter, with payloads captured from a real Codex (`tests/fixtures/codex/`), through the reducer (`tests/codex.rs`); installing, trusting and uninstalling (`src/install.rs`), including the trust hash against Codex's own test vector; linking from `CODEX_THREAD_ID`.
- **End to end, automated:** `scripts/codex-e2e.sh` runs the real Codex CLI against a stand-in model (`scripts/codex-mock-model.mjs`, which scripts the model's replies over the Responses API), with a temporary `CODEX_HOME` and `AGENT_GRAPH_HOME`. It installs the hooks with `agent-graph install codex`, runs `codex exec` **without** bypassing trust (so the trust entries are tested), and checks the graph: the session, its plan, a subagent, a command that starts another agent, and waiting.
- **The TUI, by hand against the stand-in model:** driven through a pseudo-terminal with approvals on (`approval_policy = "on-request"`), a command asking for escalation fired PermissionRequest, and the session showed as "Needs approval: <Codex's reason>" until it was interrupted (idle). The TUI's automatic session name isn't covered, since the stand-in model can't answer its naming request; reading `session_index.jsonl` is unit tested.
- **The ChatGPT app's Codex:** its own bundled binary, run as the app runs it (`codex app-server`, driven over JSON-RPC as the app is, with the stand-in model), ran the installed hooks with no bypass: the plan, the subagent and its nickname were recorded, and the viewer offered "Open in the ChatGPT app" for it.
- **By hand, with a real model** (still to do): `agent-graph install codex`, then a Codex session that makes a plan (`[tools.update_plan] enabled = true` in `~/.codex/config.toml`), spawns a subagent and runs `claude -p` from its shell; watch it in `agent-graph view`.
