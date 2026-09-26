# Agent Graph: Design

Status: draft, 2026-09-25. Builds on [idea.md](idea.md).

## 1. What we're building

Agent Graph records how AI coding sessions relate to each other and shows it as a live graph. It answers these questions for any session:

| Question | Where the answer comes from |
|---|---|
| Which session spawned which agents? | `agent.spawned` events |
| Which session spawned other sessions? | `session.started.parent` (see §6, correlation) |
| Is this session waiting on another one? | `wait.started` / `wait.ended` events |
| How much work is left before it can continue? | Worked out by the reducer (§4) |
| What's left to do inside this session? | `tasks.updated` events plus the status summary |
| What did one session send another, and what came back? | `message.sent` events with `reply_to` |

**Goals**

- Works with any provider. Nothing in the core knows about Claude, Codex, or anyone else. Each provider gets a thin **adapter**.
- Local first. Recording only ever writes files on this machine, and never touches the network. Sharing is a separate, explicit step: `agent-graph watch-remote`, or pasting a log into the site (§9).
- Zero effort for the agent. Data comes from hooks the agent can't forget to call, not from the agent choosing to report.

**Non-goals (for now)**

- Controlling agents. Agent Graph only watches; it never blocks or steers.
- Cost and token accounting. Provider OpenTelemetry already does this well.
- Storing full transcripts. We keep summaries and pointers (`transcript_path`), not content.

## 2. Architecture

```
 ┌──────────────┐  hook: JSON on stdin   ┌────────────────────┐
 │ Claude Code  │ ─────────────────────▶ │                    │
 ├──────────────┤                        │  agent-graph emit  │  append one line
 │ Codex CLI    │ ─────────────────────▶ │  (adapter per      │ ─────────────────▶ ~/.agent-graph/events/*.jsonl
 ├──────────────┤                        │   provider)        │                          │
 │ Gemini CLI … │ ─────────────────────▶ │                    │                          │
 └──────────────┘                        └────────────────────┘                          │
                                                                                         ▼
                                                                                 agent-graph view
                                                                           (reducer + local web UI, SSE)
```

All of this ships as one binary, `agent-graph`, with these subcommands:

- `emit` is called by hooks.
- `install <provider>` / `uninstall <provider>` add or remove the hooks in a provider's settings.
- `tail` draws the graph live in the terminal; `tree` prints it once.
- `view` serves the live local web UI, with a timeline you can step through.
- `snapshot` saves a phone-sized PNG (or SVG) of a session for an agent to send you; `--json` gives the raw graph.
- `watch-remote` shares the log live on the Agent Graph site and prints its link (§9).

`install` also adds an `/agent-graph` command (`src/slash.rs`), so you can ask any session for the graph:
- **Where it goes:**
  - Claude Code, Codex and Cursor get a skill (`skills/agent-graph/SKILL.md` in each client's folder).
  - Gemini CLI gets a TOML custom command.
  - All of them share one set of instructions: run `tree` and `snapshot`, send the image if the app can show files, and summarise what needs the user, what's stuck and what's in progress. `share` runs `watch-remote --session current` instead.
- **Permissions:** Claude Code's skill pre-approves only the local, read-only `tree` and `snapshot`. Sharing sends data to a website, so it keeps its permission prompt.
- **Safety:** each file carries an "Installed by agent-graph" marker. Reinstalling updates only a file with the marker, and uninstalling removes only those files. A command the user wrote themselves is left alone.
- `run -- <command>` runs any command as a node in the graph, linked to whatever started it, with anything it starts linked to it (§6).

## 3. Data model: an append-only event log

### Why a log and not a shared state file

Many sessions write at the same time. If they all rewrite one `state.json`, two writes that overlap will lose one of them. Instead:

- Each session appends events to **its own file**: `~/.agent-graph/events/<provider>-<session_id>.jsonl`. Subagent events go in their parent session's file.
- Each event is one line, written with a single `write()` to a file opened `O_APPEND`. Lines are capped at 4 KB, and longer text is cut down. On local filesystems, appends this small don't interleave, so no locks are needed.
- The current state is rebuilt by a **reducer** that replays every file. `agent-graph snapshot --json` gives simple consumers the whole graph, but that's always generated output.
- The log also gives us message history and a timeline for free.

### Event envelope

```json
{
  "v": 1,
  "id": "01J8ZK3V7Q9R2M5X4T6W8Y0B1C",
  "ts": "2026-09-25T10:14:03.221Z",
  "type": "agent.spawned",
  "node": "claude-code:5f2c1e/a3fd07",
  "parent": "claude-code:5f2c1e",
  "source": { "provider": "claude-code", "provider_version": "2.1.260", "adapter": "claude-code@1", "host": "mbp" },
  "trace": { "traceparent": "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01" },
  "data": { "agent_type": "Explore", "purpose": "Find the auth middleware" }
}
```

- `id` is a ULID, so IDs sort by time and never collide.
- `node` is the session or agent the event is about. It's namespaced by provider, so IDs from different tools can't collide:
  - `<provider>:<session_id>` for a session
  - `<provider>:<session_id>/<agent_id>` for a subagent
- `parent` is set only on events that create a node.
- `trace` is optional W3C Trace Context (see §6).

[`schema/event.schema.json`](../schema/event.schema.json) is the published contract (JSON Schema 2020-12). Tests check that everything we emit validates against it. Consumers must ignore event types and fields they don't recognise.

### Event types

| Type | `data` fields | Emitted when |
|---|---|---|
| `session.started` | `cwd?`, `source?` (startup/resume/…), `title?`, `transcript_path?`, `link_method?`, `process?`, `ancestors?` | A provider session begins or resumes. The last three link it to the session that started it (§6) |
| `session.ended` | `reason?` | A session exits |
| `agent.spawned` | `agent_type?`, `purpose?`, `background?` | A subagent starts |
| `agent.finished` | `status` (completed/failed/canceled), `summary?` | A subagent stops |
| `status` | `state`, `summary?` | The state changes (see below) |
| `tasks.updated` | `items: [{id, text, active_text?, status}]` | A todo tool sent its full list. Replaces the node's list |
| `task.upserted` | `id`, `text?`, `active_text?`, `status?` | An incremental task tool created or changed one task |
| `task.deleted` | `id` | A task was removed |
| `spawn.requested` | `call_id`, `kind` (agent/session), `agent_type?`, `purpose?`, `background` | A node asked for a child to start. Unless `background`, the node is blocked until the matching `spawn.returned`, or until it goes idle or ends (§4) |
| `spawn.returned` | `call_id`, `child?`, `outcome?` | The spawning call returned. `child`, when the provider reports it, is authoritative |
| `wait.started` | `wait_id`, `on` (node id), `reason?` | A node blocks on another node for some other reason |
| `wait.ended` | `wait_id`, `outcome?` | That block clears |
| `message.sent` | `message_id`, `to`, `reply_to?`, `summary?`, `body?` | A message goes from one node to another. `body` only with body capture on |
| `activity` | `tool`, `label?` | Optional heartbeat, one per tool call (off by default) |

Two things shape these types:

- **Adapters are stateless.** Each hook runs in its own process, often in parallel with others, so an adapter can't safely keep a side file of "tasks so far" or "spawns in flight". Anything that needs history is left to the reducer. That's why incremental task tools emit `task.upserted` rather than a rebuilt full list, and why a spawn is two events (`spawn.requested`, `spawn.returned`) that the reducer pairs with the child's `agent.spawned`.
- **Waiting on the human is a state, not a wait.** `input_required` already says who the node is waiting on, and any later activity from the node clears it.

**States** reuse the A2A protocol's task states where they fit, so we don't invent a new vocabulary:

- `working`
- `input_required`: waiting on the human
- `idle`: turn finished; the session is alive and waiting for its next prompt
- `completed`
- `failed`
- `canceled`

"Waiting on another node" isn't a state of its own: the reducer derives it from open waits (§4), so a node can be `working` and blocked at the same time.

## 4. The reducer (derived state)

The reducer replays events into this shape:

```
Node { id, kind: session|agent, provider, parent, children[], state, summary, attention,
       purpose, agent_type, background, spawned_by, tasks[], spawns[], waits[], messages[],
       started_at, ended_at, last_event_at,
       // derived
       headline, open_tasks, blocked { on[], starting, nodes, open_tasks }, stale }
```

It works out these values:

- **Spawn binding.** Each `spawn.requested` is paired with the child it started:
  - When `spawn.returned` names the `child`, that's used. Claude Code always reports it.
  - Until then (a foreground agent only returns when it finishes), the reducer guesses: a new child is paired with the oldest unpaired request in the same session, preferring one with the same agent type. The guess is corrected when the call returns.
  - A paired child takes the request's `purpose` and `background` flag, and is moved under the node that asked for it. That's how an agent started by another agent ends up nested correctly, even though the provider only reports the session as its parent.
  - When a correction takes a child away from a request it had been guessed for, that child is paired again with whichever request is left. Without this, three parallel agents that start in a different order than requested leave one running agent unmatched.
- **Waits-on edges.** Every open wait: foreground spawns (open from `spawn.requested` until the call is over, pointing at the child once paired) and explicit `wait.started` events.
  - A spawn's wait ends when its call returns. The child finishing doesn't end it, because which request that child answers may still be a guess; ending it then would close the wrong request's wait.
  - Claude Code only reports a call's return when it succeeds. So a node going idle (its turn is over) or ending (including an agent canceled with its session) also ends every foreground call it made: their waits close, and they can't be paired with a later child.
  - A node that has finished isn't waiting on anything.
  - Explicit waits end with `wait.ended`, or when their target finishes.
  - A wait whose target has finished never counts as blocking, even if the event that formally ends it is missing.
- **Deadlock.** If following a node's waits (and the children of what it waits on) leads back to the node itself, nothing in that loop can finish: `blocked.cycle` is set, and every view shows it.
- **Blocked chain.** Follow the waits-on edges from a node until you reach a node that isn't waiting.
- **Work remaining before X can continue.** Walk the waits-on graph from X, recursively. Count the open tasks in every node you reach, including their children. Show it like this:

  > "waiting on 2 nodes (7 open tasks)"

  We count tasks, not minutes. Any time estimate would be a guess dressed up as data.
- **Staleness.** A node is `stale?` when it's `working`, isn't waiting on anything, and has had no events for N minutes (30 by default). Hooks don't fire when a process crashes or hangs. The rule is deliberately narrow:
  - `idle` and `input_required` nodes are legitimately quiet.
  - A node blocked on another is waiting, not hung. The views point at the stale node it's waiting on ("Explore … looks stuck").
- **The clock.** Staleness and "last 24 hours" are measured against now, or against `AGENT_GRAPH_NOW` when set, so saved logs such as the examples read the same whenever they're opened. When viewing a past step, "now" is that step's time.
- **What's left, concisely.** The `summary` line, plus the in-progress task's text, plus "*n* more pending". An optional LLM-written one-line summary is a later, opt-in viewer feature (§11).

## 5. How hooks work

Hooks are the only way data gets in, so this section goes into detail.

### 5.1 The idea

Most AI coding CLIs now let you register **shell commands that run at fixed points in the agent's lifecycle**: session start, before and after each tool call, when a subagent starts or stops, when the agent finishes a turn, and so on. The provider handles each point the same way:

1. It pauses at that point (or doesn't, for background hooks).
2. It starts your command as a child process.
3. It writes one **JSON object describing the event to the command's stdin**.
4. It reads the command's **exit code** and **stdout**, which can allow, block, or add context. We never use this.

Hooks are well suited to Agent Graph for three reasons:

- **They're deterministic.** They fire every time. An agent that is supposed to call a "report status" tool will often forget, and hooks don't depend on it.
- **They're out of band.** They don't use the model's context window and need no prompt changes.
- **The pattern is nearly universal.** Claude Code, Codex CLI, Gemini CLI, Cursor, GitHub Copilot CLI, Cline and Windsurf all use "run a command with JSON on stdin". OpenCode and Amp use JS/TS plugins instead, which can do the same thing.

What hooks are *not*: a standard. Event names, field names (snake_case vs camelCase), and how parents are identified all differ between providers. That's why each provider needs its own adapter.

### 5.2 A concrete walkthrough (Claude Code)

**Configuration.** Hooks live under a `hooks` key in `~/.claude/settings.json` (user), `.claude/settings.json` (project), or `.claude/settings.local.json` (project, not committed). A Claude Code **plugin** can also ship them in `hooks/hooks.json`, which may be the cleanest way to distribute ours later. The structure is event → list of matcher groups → list of handlers. This is what `agent-graph install claude-code` writes (the command is the full, quoted path of the installed binary):

```json
{
  "hooks": {
    "SessionStart":     [{ "hooks": [{ "type": "command", "command": "agent-graph emit --provider claude-code", "timeout": 10 }] }],
    "SessionEnd":       [{ "hooks": [{ "type": "command", "command": "agent-graph emit --provider claude-code", "async": true }] }],
    "UserPromptSubmit": [{ "hooks": [{ "type": "command", "command": "agent-graph emit --provider claude-code", "async": true }] }],
    "Stop":             [{ "hooks": [{ "type": "command", "command": "agent-graph emit --provider claude-code", "async": true }] }],
    "Notification":     [{ "hooks": [{ "type": "command", "command": "agent-graph emit --provider claude-code", "async": true }] }],
    "SubagentStart":    [{ "hooks": [{ "type": "command", "command": "agent-graph emit --provider claude-code", "async": true }] }],
    "SubagentStop":     [{ "hooks": [{ "type": "command", "command": "agent-graph emit --provider claude-code", "async": true }] }],
    "PreToolUse": [{
      "matcher": "Agent|Task|AskUserQuestion|ExitPlanMode|Bash",
      "hooks": [{ "type": "command", "command": "agent-graph emit --provider claude-code", "async": true }]
    }],
    "PostToolUse": [{
      "matcher": "Agent|Task|TaskCreate|TaskUpdate|TodoWrite|SendMessage|AskUserQuestion|ExitPlanMode|Bash",
      "hooks": [{ "type": "command", "command": "agent-graph emit --provider claude-code", "async": true }]
    }]
  }
}
```

- `matcher` filters by tool name for tool events, or by a sub-type for others; for example, `SessionStart` can match `startup|resume|clear|compact`. A plain `A|B` is an exact list. Anything with regex characters is treated as an unanchored regex.
- We match only the tools that matter to the graph, because **every match starts a process**. A heartbeat mode (`--activity`) can match `*` for users who want it.
- `Bash` is in both tool matchers, so we can see when a session launches another agent from its shell (see below). For any other shell command `emit` writes nothing, but it still costs a background process per call. Claude Code's per-handler `if` filter (permission-rule syntax, e.g. `Bash(codex *)`) may let us skip those processes entirely; to verify.
- `"async": true` runs the hook in the background, so the agent never waits for us.
- `SessionStart` stays synchronous, so it's recorded before anything else in the session, and because it must write the session's identity for its shell before the first shell command runs (see §6). It's also a hook whose stdout Claude Code **adds to the model's context**, which is one more reason `emit` must never print anything.
- `install` keeps everything else in the file, backs the old file up to `settings.json.agent-graph.bak`, asks before writing (or takes `--yes`), and offers `--dry-run`. Installing twice changes nothing; `uninstall` removes only our handlers.

**The payload.** Every event includes common fields: `session_id`, `transcript_path`, `cwd`, `hook_event_name` and `permission_mode`. When the hook fires *inside a subagent*, it also gets `agent_id` and `agent_type`, and `session_id` stays the parent session's ID. Here's an illustrative `SubagentStart` payload on stdin:

```json
{
  "session_id": "5f2c1e…",
  "transcript_path": "/Users/you/.claude/projects/-Users-you-app/5f2c1e….jsonl",
  "cwd": "/Users/you/app",
  "hook_event_name": "SubagentStart",
  "agent_id": "a3fd07…",
  "agent_type": "Explore"
}
```

The adapter turns that into:

```json
{"v":1,"type":"agent.spawned","node":"claude-code:5f2c1e…/a3fd07…","parent":"claude-code:5f2c1e…","data":{"agent_type":"Explore"}, …}
```

**Mapping for Claude Code** (as implemented in `src/adapter/claude_code.rs`, checked against payloads recorded from Claude Code 2.1.118):

| Claude Code hook | Agent Graph event |
|---|---|
| `SessionStart` (`source`: startup/resume/clear/compact) | `session.started`. A new session is `idle` until its first prompt |
| `UserPromptSubmit` | `status: working` |
| `PreToolUse` on `Agent`/`Task` | `spawn.requested` with the call's `tool_use_id`, `description` as the purpose, `subagent_type`, and `run_in_background` |
| `SubagentStart` | `agent.spawned` for `<session>/<agent_id>` |
| `SubagentStop` | `agent.finished`. `last_assistant_message` becomes the summary only with body capture on |
| `PostToolUse` on `Agent`/`Task` | `spawn.returned` with `child` from `tool_response.agentId`. Background agents return at once (`status: async_launched`), foreground ones when they finish; both report `agentId`, so the pairing is exact |
| `PostToolUse` on `TaskCreate` / `TaskUpdate` | `task.upserted` (the new id comes from `tool_response.task.id`), or `task.deleted` |
| `PostToolUse` on `TodoWrite` | `tasks.updated` with the full list |
| `PostToolUse` on `SendMessage` | `message.sent` with `to`, `summary` and `msg_id`. The body only with body capture on |
| `PreToolUse` on `Bash`, when the command starts another agent | `spawn.requested` with `kind: session`, the program as `agent_type`, the call's `description` as the purpose, and `background` for `&` or `run_in_background` |
| `PostToolUse` on `Bash`, for the same command | `spawn.returned`, without a `child`: the reducer pairs it (below) |
| `PreToolUse`/`PostToolUse` on any other `Bash` command | Nothing |
| `Notification`: `permission_prompt`, `agent_needs_input`, `elicitation_dialog`, `elicitation_url_dialog` | `status: input_required`, with the notification's message |
| `Notification`: `idle_prompt` | `status: idle` |
| `PreToolUse` on `AskUserQuestion` | `status: input_required`, e.g. "Asks: Which database should the cache use?" |
| `PreToolUse` on `ExitPlanMode` | `status: input_required`, "Plan ready for your review" |
| `PostToolUse` on `AskUserQuestion` / `ExitPlanMode` | `status: working` (answered) |
| `Stop` | `status: idle` (the turn is over) |
| `SessionEnd` | `session.ended` |
| Any other hook | `unknown`, with just the hook and tool names |

Hooks that fire inside a subagent carry its `agent_id`, so their events land on the subagent's node rather than the session's.

Claude Code migrated from `TodoWrite` to `TaskCreate`/`TaskUpdate`, but headless `claude -p` sessions still only offer `TodoWrite`, so the adapter handles both. That's the general lesson: **adapters must tolerate payloads changing between provider versions and modes.**

**Waiting on a separate session.** When session A runs another agent CLI from its shell *in the foreground*, hooks alone show the whole exchange (checked with Claude Code running `claude -p` from its shell):

1. A's `PreToolUse(Bash)` fires. The command starts a known agent CLI, so we emit `spawn.requested` with `kind: session`. A is now waiting on a session that's "starting".
2. The child starts. Its own `SessionStart` hook links it to A (§6), and the reducer pairs it with A's oldest unpaired session request made before it started, preferring one for the same program (`claude` for a `claude-code` session). A is now waiting on it.
3. A's `PostToolUse(Bash)` fires when the child exits. That's the `spawn.returned`, and the wait ends. (If the command fails, `PostToolUse` doesn't fire; the wait ends when A's turn does.)

How commands are recognised (`adapter/shell.rs`):
- The command is split into simple commands at `;`, `&&`, `||`, `|`, `&`, newlines, parentheses and command substitutions (even inside double quotes), but not inside quotes. Each one's program is found past `NAME=value` assignments, shell keywords, and wrappers like `env`, `nohup`, `timeout 600` or `npx`.
- Known agents: `claude`, `codex`, `gemini`, `cursor-agent`, `copilot`, `opencode`, `amp`, `aider`, `goose` and `qwen`, plus any in `AGENT_GRAPH_AGENT_COMMANDS`. `agent-graph run -- X` counts as starting X.
- Not counted: `--version` or `--help`, and subcommands that manage an agent (`claude mcp …`, `codex login`, …).
- A trailing `&` (or the tool's `run_in_background`) makes it a background spawn, so A isn't blocked. A background launch returns before its child starts, so the reducer still pairs those after they've returned.

We don't record the prompt or the output: with body capture off, prompts are never kept (§5.3 rule 5), and the Bash tool's `description` is a better label anyway. A script that starts an agent inside itself isn't seen as a launch, but the agent still links itself under the session.

### 5.3 Rules for `agent-graph emit`

The hook runs on the agent's critical path, possibly hundreds of times per session, so it follows strict rules:

1. **Always exit 0 and never print to stdout.** Exit code 2 would *block* the agent's action, and stdout can be fed into the model's context. Errors, and even panics, go to `~/.agent-graph/emit.log`. For the same reason `emit` doesn't go through the normal argument parser, which exits with code 2 on a usage error.
2. **Be fast.** Target under 10 ms: parse stdin, normalize, do one `write()`, exit. Measured at about 3.6 ms per hook on an M-series Mac, including process start-up. That's why the binary is Rust rather than Node or Python, since start-up time dominates.
3. **No network, ever.** `emit` only appends to a local file.
4. **Don't crash on unknown input.** An unknown hook is recorded as an `unknown` event with just its name. Input that can't be used (not JSON, no session id) is logged and dropped.
5. **Redact by default.** Prompts and tool inputs can contain secrets. We keep short labels (at most 200 characters), such as the subagent's `description`, task text and message summaries, and drop full prompts, tool outputs, message bodies and final agent messages unless `AGENT_GRAPH_CAPTURE_BODIES=1`.
6. **Keep a raw capture mode.** `AGENT_GRAPH_RAW=1` saves the untouched payloads to `raw/`. That's how we build and regression-test adapters: record real payloads, commit them as test fixtures, and check that adapters produce the expected events.

### 5.4 Provider support (research as of 2026-09)

| Provider | Mechanism | Payload | Subagent + parent link | Todo list | Waiting signals |
|---|---|---|---|---|---|
| **Claude Code** | Command hooks in `settings.json` or plugin `hooks/hooks.json` | stdin JSON, snake_case | `SubagentStart`/`Stop` with `agent_id`; `session_id` is the parent's | `TaskCreate`/`TaskUpdate` (legacy `TodoWrite`) via `PostToolUse` | `Notification` (permission/idle), `Stop` |
| **Codex CLI** | `~/.codex/hooks.json` or `[hooks]` in `config.toml`, plus project `.codex/` (trusted projects only) | stdin JSON, snake_case; adds `turn_id`, `model` | `SubagentStart`/`Stop` with `agent_id`; docs say `session_id` is the parent's | Plan tool via `PostToolUse` (verify) | `PermissionRequest`, `Stop`, `Interrupt` |
| **Gemini CLI** | `hooks` in `.gemini/settings.json` | stdin JSON; `GEMINI_SESSION_ID` env var | **No subagent events.** Subagents are tools named after the agent, so we infer them from `BeforeTool`/`AfterTool` | Todo tool via `AfterTool` (verify) | `Notification` (`ToolPermission`), `AfterAgent` |
| **Cursor** | `.cursor/hooks.json` / `~/.cursor/hooks.json` | stdin JSON; `conversation_id`, `generation_id` | `subagentStart` has `parent_conversation_id` and `tool_call_id`, the best parent data of any tool here | Unknown | `stop` with `status` |
| **Copilot CLI** | `.github/hooks/*.json`, `~/.copilot/hooks/` | stdin JSON, **camelCase** | `subagentStart`/`subagentStop` | Unknown | `notification`, `permissionRequest` |
| **OpenCode** | JS/TS plugin (`.opencode/plugins/`) | Event stream | Sessions have `parentID` | `todo` events | `permission.asked`, `session.idle` |
| **Amp** | Bun TS plugin (`.amp/plugins/`) | Event stream | `parentThreadID` | Unknown | Unknown |
| **Cline / Windsurf** | Command hooks | stdin JSON | Task/trajectory only, no subagents | No | Partial |
| **Aider** | No hooks | — | — | — | Wrapper only (`agent-graph run -- aider`) |

Known gaps to check by hand:

- Whether Cursor's CLI (`cursor-agent`) fires hooks at all; sources disagree.
- Whether Codex hooks fire under `codex exec`.

For plugin-based tools (OpenCode, Amp), the adapter is a ~50-line plugin that calls `agent-graph emit` or writes the JSONL line directly.

### 5.5 What hooks can't tell us

Hooks show what the **provider** does, not what the **model** means. We accept these gaps:

- **Waits that exist only in the model's plan.** For example, session A tells the user "I'll wait for the Codex run in the other terminal", or polls a file another session writes. There's no mechanical signal, so both sessions appear in the graph but without a waits-on edge.
- **A free-form status line.** We use the in-progress task's text instead. Where a payload includes the agent's last message (`SubagentStop` has `last_assistant_message` in both Claude Code and Codex), it can become the `agent.finished` summary when body capture is on.
- **Messages between tools with no messaging.** If a provider has no way for sessions to talk, there's nothing to record. Agent Graph observes; it doesn't add a messaging feature.

An MCP server that lets agents report these things themselves could be added later without changing the event format. We left it out because it depends on the model remembering to call it, which is the weakness hooks avoid, and because it would add a component and an install step per provider for small gains.

## 6. Correlation: linking sessions across processes

Subagents are easy because the provider tells us the parent (§5.2). **Separate sessions** are harder. Examples: Claude running `codex exec …` from its shell tool, or a script launching three `claude -p` workers. We use these methods in order. The reducer records which one linked each session (`node.link`: `env`, `run` or `process`), and the viewer shows it as "Linked by".

1. **Environment propagation (preferred).** When a session starts, `emit` passes its identity to the agent's shell, so any process started from that shell inherits it. The child's own `SessionStart` hook reads it back, and its `session.started` gets that `parent` (`data.link_method: env`).
   - Variables: `AGENT_GRAPH_PARENT=<node id>`, plus a W3C `TRACEPARENT` following OpenTelemetry's environment-variable spec for passing trace context to child processes. The child continues the parent's trace, and its own `traceparent` goes in its envelope's `trace`.
   - Claude Code: the `SessionStart` hook appends `export …` lines to the file named by `$CLAUDE_ENV_FILE`, and Claude Code applies them to the session's later shell commands. Checked with Claude Code in September 2026: a `claude -p` started from a session's shell linked itself to it and continued its trace.
   - Adapters say which variable names such a file (`Adapter::env_file_var`). Other providers need the equivalent; where none exists, use method 2 or 3.
   - A value that isn't a well-formed node id, or is the session's own id (its own later hooks may see it), is ignored.
2. **Wrapper.** `agent-graph run -- <any command>` (`run.rs`) is a node of its own (`run:<ulid>`, named after the program or `--name`), linked to whatever started it, and it sets the variables for the command. It's working until the command exits, then completed, or failed with the exit code, which it passes on. This works for any tool, including ones with no hooks at all, and groups the sessions a script starts. It waits out Ctrl+C (which the command gets too) so its end is always recorded, and passes on a `kill` or hang-up sent to it alone. A signal it was started with ignored (under `nohup`, or as a background job) stays ignored, for it and the command.
3. **Process tree (fallback).** `session.started` records the agent's process and the processes above it (`process.rs`), each as `<pid>@<start time>`, so a reused pid never matches.
   - The agent is the nearest process above the hook that isn't a shell. The walk stops at the top, at a process it can't read, or at a "parent" that started after its child (its pid was reused).
   - Linux reads `/proc`, macOS `proc_pidinfo`, and Windows a Toolhelp snapshot with `GetProcessTimes`. Elsewhere nothing is recorded.
   - The reducer links a session with no `parent` to the session whose agent process is its nearest ancestor (`link: process`), which catches a child that didn't inherit the environment. It needs no provider support, and fails for detached processes.
   - A `/clear` starts a new session in the same process. The newer session owns the process from then on, and neither is the other's parent.

We use the standard W3C `TRACEPARENT` format rather than inventing our own, so the IDs stay compatible with OpenTelemetry tooling.

A session a session started is a process of its own: it reports its own end, so it isn't canceled when its parent ends (a subagent is).

## 7. Storage

```
~/.agent-graph/                       # or $AGENT_GRAPH_HOME; %USERPROFILE%\.agent-graph on Windows
  events/<provider>-<session>.jsonl   # source of truth, append-only
  raw/                                # only with AGENT_GRAPH_RAW=1
  images/                             # PNGs from `agent-graph snapshot`
  emit.log                            # emit errors; starts over after 1 MB
```

On Unix the directory is created `0700` and files `0600`. On Windows the user profile's permissions already keep it private.

Settings are environment variables for now, since hooks inherit the agent's environment:

| Variable | Effect |
|---|---|
| `AGENT_GRAPH_HOME` | Use a different data directory |
| `AGENT_GRAPH_RAW=1` | Also save every raw hook payload to `raw/` |
| `AGENT_GRAPH_CAPTURE_BODIES=1` | Keep message bodies and final agent messages |
| `AGENT_GRAPH_NOW` | When reading logs, pretend it's this time (RFC 3339). Never affects recording |

A `config.toml` can come later if settings outgrow this. Retention (`agent-graph gc --older-than 14d`) is also still to do.

## 8. Viewing the graph

There are four views of the same graph. All of them flag the same three kinds of trouble:
- **Needs you:** `input_required`.
- **Stale?:** looks hung.
- **Deadlock.**

### `agent-graph tail`: live, in the terminal

A full-screen tree, redrawn as events arrive, like `top`:
- A summary line on top ("16 sessions · 19 working · 113 events · 2 need you · 3 stale? · 2 deadlocked").
- States in colour. `--ascii` switches to plain branch characters.
- `q` quits and `a` toggles older sessions. The terminal is restored however it exits.

When stdout isn't a terminal, it prints a fresh frame on each change instead, so it can be piped or logged. It uses `crossterm`, which handles raw mode and keys the same on Windows, macOS and Linux.

### `agent-graph tree`: once, as text

```
claude-code:e0000002  search-indexer  [working]  Rebuilding the index schema (+1 pending)  tasks 0/2  waiting on 1 node (0 open tasks), Explore a91d0160 looks stuck
└─ Explore a91d0160  [working] (stale?)  Profile the slow bulk import
```

### `agent-graph view`: live, in the browser

Serves port 7777, and prints its link with this run's key, `http://127.0.0.1:7777/?key=…` (`--open` opens it). The layout:
- **Sessions sidebar.** Each entry shows what's happening. Anything that needs you, is deadlocked or looks stuck is called out there, so trouble is visible without clicking.
- **Tree.** Cards coloured by state, with task progress and what each node is waiting on.
- **Detail panel.** Tasks, waits, agents it started, messages and timestamps for the selected node.
- **Timeline.** Every event in the selected session is a stop on a slider, with ticks coloured by kind (lifecycle, agents, tasks, needs you, messages).
  - Drag, click, or use ←/→ to step. The graph re-renders as it was at that moment, and the node that step changed is ringed.
  - The last stop is "live". New events extend the timeline without moving you if you're looking at the past.
- **Save image** downloads a PNG of the session as shown, at the step being viewed.
- **Resume in Claude Code**, in a session's details, reopens it in a new terminal window.
  - It runs `claude --resume <id>` in the session's folder, which is where Claude Code files it. When the agent exits, the window is left with a shell in that folder.
  - A session that hasn't ended is probably still open in another terminal, and two processes on one conversation would both write to it. So for those the button says **Open a copy** and adds `--fork-session`, which branches the conversation instead.
  - The window: macOS opens a one-off `.command` script (in Terminal, or whatever the user has chosen for those), which deletes itself. Windows uses `start` to run `cmd /K`. Linux uses `$TERMINAL`, or the first common terminal it finds. If none works, the page shows the error and the command to run by hand.
  - Codex and others get the button when their adapters land (`codex resume <id>`).
  - Only sessions on this computer can be reopened, so the shared site never offers it (below).

How it works:
- **One graph implementation.** A thread tails the event files (reading only new, complete lines) and tells pages about changes over Server-Sent Events. Pages then ask for the graph again, live or `?until=<event id>`, so the Rust reducer stays the only implementation of the graph logic. There's no JavaScript copy to drift out of step.
- **Security.**
  - The server listens only on `127.0.0.1` and `[::1]` (when there's IPv6).
    - Browsers try `localhost` as `[::1]` first, so if another program were listening there, a `localhost` link would reach it. The printed link names `127.0.0.1`, and the viewer won't start if `[::1]` at its port is already taken, so a typed `localhost` is safe too.
  - That keeps other computers out, but not other accounts on this one. So each run makes a random 160-bit key and prints its link with `?key=<key>`.
    - The page, its script and its styles are built in and private to no one, so they're served to anyone. Everything under `/api/` needs the key, compared in constant time.
    - The page takes the key from the link, keeps it in its own origin's `localStorage` (so a reload or a second tab works), takes it out of the address bar, and sends it with every API request as an `X-Agent-Graph-Key` header. Another website can't send that header without a CORS preflight the server never grants. The event stream, which can't set headers, carries it as `?key=`; nothing else accepts it that way. Save image fetches the picture with the header and saves it from memory, so the key isn't recorded in the file's "where from" details.
    - It isn't a cookie: browsers send cookies to every port on `localhost`, so any other local web server (another account's included) would receive it.
  - It answers only requests whose `Host` is `localhost`, `127.0.0.1` or `[::1]` on its port, which stops other websites reaching it through DNS rebinding.
  - At most 256 connections are served at once (each open page holds one for its event stream), and a request's head must arrive within 5 seconds, so a slow or flooding client can't tie up every thread. A local user without the key can still lock the viewer out for as long as they keep opening connections, but can't read anything.
  - It sends a strict Content Security Policy (`default-src 'none'`, same-origin scripts, styles and fetches only).
  - It loads nothing from the internet.
  - Resuming runs a program, so it gets more care:
    - It's a `POST /api/open?node=<id>`, answered only when the `Origin` header is the viewer's own. Other websites can send requests to localhost too, but browsers always say where a POST came from.
    - Only the node id comes from the page. The command is worked out from the log (`resume.rs`), and only for session ids made of letters, digits, `-` and `_` that start with a letter or digit, so a crafted log can't slip in an option like `--dangerously-skip-permissions`.
    - The folder never reaches a shell's parser unquoted: it's the working directory on Windows, a separate argument on Linux, and single-quoted in the macOS script.
    - Before launching, the server checks the folder exists and, when the log names one, that the session's transcript does too. Otherwise it says the session isn't on this computer.

### `agent-graph snapshot`: an image to send

Renders a session as a PNG sized for a phone (1080 px wide) and prints only its path, so an agent can run it and pass the file to you. The picture has:
- a header with any needs-you callouts,
- the tree of agents,
- the session's task list.

Which session it draws:
- `--session <id prefix>` or `--all` if given.
- Otherwise the Claude Code session it runs inside (`CLAUDE_CODE_SESSION_ID`, which matches the hooks' `session_id`).
- Otherwise the most recent session in the current folder, then the most recent overall.

It's drawn as SVG and rasterized with `resvg`, pure Rust with the system's fonts, so it looks the same on every platform. `--out x.svg` keeps the SVG. `--json` (or `--out x.json`) gives the raw graph instead.

### Text from models is untrusted

Task text, purposes, summaries and messages are written by models, so every view treats them as data:
- **Web page:** inserted only as text nodes, never as HTML.
- **SVG:** escaped.
- **All views:** control characters become spaces, which stops terminal escape sequences and newlines. Bidirectional overrides are dropped, which stops text displaying differently from what it is.

The `hostile-text` example checks all of this.

## 9. Sharing on the web

The site (`site/`, deployed at `https://agentgraph.chofter.com`) is a pastebin for Agent Graph logs. A log can get there two ways, and either way you get a permanent link to a viewer:
- **Paste or upload** it on the home page.
- **Stream it live** with `agent-graph watch-remote`.

Viewers can step through the log exactly as they can locally. Each log can have a password.

**One implementation, again.** The site's viewer is the same page as `agent-graph view` (`app.js`, `app.css`, and the markup from `index.html`):
- `app.js` reads through a small data-source interface. By default that's the local server's API.
- On the site, `site-source.js` supplies it instead. It fetches the log's text, then computes graphs and timelines in the browser with the Rust reducer compiled to WebAssembly (the `wasm/` crate: 300 KB, 100 KB gzipped).
- Graph requests say where they'll be shown: `timeline::graph` takes an `Environment`, and the WebAssembly `graph` request requires `"env"`. Only `"local"` lists the sessions that can be reopened; the site sends `"site"`, and its data source has no `open` either, so the Resume button never appears there.
- The crate builds without its CLI dependencies (`--no-default-features`), and the WebAssembly boundary is plain bytes in memory, with no binding generator.
- `site/scripts/sync-viewer.mjs` copies the viewer and the `.wasm` into the site, and CI checks the copies are current.

**The protocol** (`src/remote.rs`, `site/app/api/logs/`) is designed so the site does as little work as possible per update:

1. `POST /api/logs` with the first chunk creates the log and returns its id, public URL and a **write key**. `watch-remote` prints the URL at once.
2. Each later chunk is `POST /api/logs/{id}/append?offset=<bytes sent so far>`, carrying `Authorization: Bearer <write key>`. The client:
   - sends only whole new lines, at most 256 KB per request;
   - batches whatever arrived in the last second;
   - on network errors, keeps the lines and retries with backoff;
   - stops on a refusal.
3. The site handles an append like this:
   - checks the size from the header;
   - verifies the key by recomputing an HMAC of the id, with no database read;
   - stores the raw body, never parsed, as one new Firestore document, `logs/{id}/chunks/{offset}`.

   Nothing already stored is read or rewritten, so an append costs the same however big the log is. A retried chunk lands on the same document, and the viewer drops any event it has already seen.
4. Viewers read `GET /api/logs/{id}/content?after=<last chunk key>`. For a live log they poll every 3 s while events are arriving, backing off to 15 s when quiet or when the tab is hidden.

**Who can do what:**
- **Only the creator can add to a log.** The write key comes back only from the create call, the CLI and the upload page keep it only in memory, and every append must present it. It's derived from the site's secret, so it can't be guessed or forged, and it isn't stored anywhere. There's no API to change or delete a log.
- **Viewing** needs only the link, unless a password was set:
  - `--password=…` (or the upload page's field) is sent base64url-encoded in a header, over HTTPS; the CLI refuses a password over plain HTTP except to localhost.
  - The site stores only a scrypt hash.
  - Unlocking sets an HttpOnly cookie, derived the same way, for that log.
  - The page and the content API both check it, so the log's text never reaches a browser without the password.
- **Saved passwords.** `--save-default-password` saves the password as the default for later runs, in `~/.agent-graph/remote.json`, readable only by you. It's stored in plain text, because the site needs the password itself. `--password= --save-default-password` clears it.
- **Firestore** rules deny all direct access; only the site's server, using the Admin SDK, reads or writes.
- **Encrypted at rest.** On top of Google's disk encryption, every chunk is encrypted before it's stored, so Firestore holds only ciphertext. Someone who can read the database (console, exports, a leaked service-account key) can't read the logs.
  - AES-256-GCM, with a per-log key derived from `AGENT_GRAPH_ENCRYPTION_KEY`, a key separate from the signing secret.
  - Each chunk is bound to its log and offset, so tampering, swapping or reordering is detected.
  - Metadata and chunk sizes aren't hidden.
  - The server holds the key, so this protects stored data; it isn't end-to-end encryption.

**Live vs pasted.**
- A streamed log is judged against the viewer's clock, so a share that stopped long ago shows as stale.
- A pasted log is shown as it stood at its last event, and its "live" end is labelled "Latest".

## 10. Implementation choices

- **Language: Rust.**
  - Native binaries for Windows, macOS and Linux from one codebase. CI runs the tests on all three.
  - Starts fast enough for a hook: about 3.6 ms per `emit`, including process start-up. The release binary is about 1.1 MB.
  - Platform differences are small and kept in `paths.rs` and `store.rs`: the home directory (`USERPROFILE` vs `HOME`), Unix-only file permissions, and file names safe on Windows.
  - Dependencies: `serde`/`serde_json`, `clap` (not used by `emit`), `ulid`, `humantime`, `resvg` (images), `crossterm` (`tail`), `ureq` with rustls (`watch-remote`), and `libc` (Unix) or `windows-sys` (Windows) for the process tree. The release binary is about 3.6 MB; `emit` still runs in about 3.6 ms because none of the image or terminal code loads on that path.
- **Code layout:**
  - `event.rs`: envelope and payload types.
  - `adapter/`: one module per provider.
  - `emit.rs`, `store.rs`, `reducer.rs`, `clock.rs`.
  - `link.rs` (parent and trace variables), `process.rs` (the process tree), `run.rs` (`agent-graph run`), `adapter/shell.rs` (spotting agent launches in shell commands).
  - `timeline.rs`: the graph at any moment, and timeline stops. Shared by the local viewer and the site.
  - `resume.rs`: the command that reopens a session in its agent.
  - `render.rs` (text), `live.rs` (`tail`), `image.rs` (`snapshot`), `remote.rs` (`watch-remote`).
  - `view/`: the local web server (`open.rs` opens terminal windows), plus the page's HTML/CSS/JS embedded in the binary.
  - `install.rs`, `slash.rs` (the `/agent-graph` command), `cli.rs`.
  - `help.rs`: the help text, generated at compile time (below).
  - Everything except the core (`event`, `reducer`, `timeline` and friends) sits behind the default `cli` feature, so the core also builds for WebAssembly (`wasm/`).
- **Site:** Next.js on Vercel, with Firestore through the Admin SDK (§9, `site/README.md`).
- **Help text: one source.**
  - All of it (the overview, every command, option and example) is in `docs/cli-help.json`, as blocks: text, headings, lists, steps, two-column tables and code.
  - `build.rs` lays it out for an 80-column terminal and writes Rust constants that `cli.rs` hands to clap. There are no doc comments on the CLI types, so there's no second copy to go stale.
  - The site's `/docs` page renders the same file as HTML (the site's `dev`, `build` and `typecheck` scripts copy it into `site/lib/`; the copy isn't committed), and the home page takes its `watch-remote` options from it.
  - A missing command in the file fails the build (its constants don't exist). A test fails if an option has no text, or if the file describes a command or option the CLI doesn't have.
- **Schema:** JSON Schema for the envelope and each event type, in `schema/`. The schema is the real "open standard" part of the project: any provider can emit events without our code.
- **Tests:**
  - unit tests per module;
  - fixture-driven adapter tests, including payloads recorded from a real Claude Code session;
  - reducer tests;
  - a schema test that validates everything we emit;
  - tests that run the real binary, the way hooks do, and that talk to the viewer over real sockets;
  - linked sessions (`tests/sessions.rs`), and `emit` and `run` through the real binary;
  - the example logs (`examples/`): 23 sessions of normal work and edge cases, with a test per scenario.
- **Distribution:**
  - For now, `cargo install --path .`. Prebuilt binaries per platform and package managers (Homebrew, winget/Scoop) come later.
  - `agent-graph install claude-code` safely merges hook config into the provider's settings (§5.2). Other providers' installers come with their adapters.
  - Possibly a Claude Code plugin and a Codex plugin that ship the hooks.

## 11. Roadmap

| Phase | Scope | Result | Status |
|---|---|---|---|
| **0** | JSON Schema, event envelope, reducer with fixture tests | The contract is fixed | Done |
| **1** | `emit` + Claude Code adapter + `install claude-code` + raw capture, plus `tree` | Real events flowing | Done; verified with a live Claude Code 2.1.118 session |
| **2** | `view` (live web UI with timeline), `tail`, `snapshot` images, example logs | Most of the idea doc's questions answered for Claude Code | Done |
| **2b** | Sharing site (paste, upload, `watch-remote`, passwords), viewer via WebAssembly | Share a live graph by link; view it on a phone | Done; tested against the Firestore emulator |
| **2c** | An `/agent-graph` command installed with the hooks (a skill in Claude Code, Codex and Cursor; a TOML command in Gemini CLI): a snapshot plus a summary, or a live link | Check on agents from any session, or a phone | Done; tested with a live Claude Code session |
| **3** | Codex adapter, then Gemini | Proves the design works across providers | |
| **4** | Correlation methods 1–3, `agent-graph run`, shell-launched session waits | Cross-session links and waits | Done; checked with Claude Code starting `claude -p` from its shell |
| **5** | Cursor/Copilot/OpenCode adapters, optional LLM summaries, `gc` | Wider coverage | |

## 12. Open questions and risks

- **Payload drift.** Providers change hook payloads between versions. Mitigations: tolerant adapters, `source.adapter` on every event, and raw-capture fixtures in the tests. The first real one is `tests/fixtures/claude-code/live-2.1.118-todowrite.jsonl`. Claude Code's hook payloads don't include its version, so `source.provider_version` is empty for now.
- **Hook overhead.** Measured in phase 1 at about 3.6 ms per hook. If that ever matters, a Unix-socket fast path to a resident `view` process is the fallback.
- **Pairing a spawn with its child.** *Resolved for Claude Code:* `PostToolUse(Agent)` reports `agentId`, which matches `SubagentStart`'s `agent_id` (checked in a live session). The reducer's guess only matters while a foreground agent is still running, and can briefly pair parallel agents of the same type the wrong way round until they return.
- **Pairing shell-launched sessions.** Nothing names the child when a shell command returns, so the reducer guesses: the oldest unpaired request, preferring the same program. Two parallel launches of the same program can be paired the wrong way round. The child's ancestors include the shell each call ran in, but the `PreToolUse` hook runs in a different process and can't see that shell's pid, so this can't be settled yet.
- **Crashed sessions** never emit `session.ended`. The staleness rule covers this for now. `session.started` now records the agent's process, but a liveness check also needs to know the log came from this machine (a shared or copied log's processes would all look dead), so it waits for a host id in `source`.
- **False "stale?" during long, quiet work.** We only hook the tools that matter to the graph, so an agent that spends 30+ minutes in `Bash`, `Edit` and `Read` calls produces no events and gets flagged, even though it's healthy. Likewise, one long-running command (a 40-minute build) is silent however we hook it. A fix: a lightweight `activity` event on `PreToolUse`/`PostToolUse` for every tool. The node could then show "running `npm test` for 12m", and a node inside a tool call wouldn't be flagged. That's one extra background process per tool call (about 4 ms, never blocking), plus some log growth. Until then, `--stale-minutes` is the knob.
- **Clock skew between hooks.** Events are ordered by the time each hook process started. Background hooks that start within a millisecond of each other could be recorded out of order. The reducer tolerates the likely cases (e.g. a child starting before its request is seen), but a per-session sequence number may be needed if this shows up in practice.
- **Privacy.** Redaction defaults must be conservative. Task text and subagent descriptions can still contain sensitive details, so `~/.agent-graph/` is created readable only by the current user.
- **Terminal widths.** `tail` cuts lines by character count, so wide (CJK) characters can overflow a line. A `unicode-width` dependency would fix it if it matters.
- **Fonts on minimal Linux.** `snapshot` uses system fonts. On a machine with none installed (some containers), text in the image is missing. Bundling one open font would remove that dependency.
- **The public site.** Nothing limits how many logs one client creates, and there's no way yet to delete a shared log. Both are needed before the site is widely used. Rotating `AGENT_GRAPH_SECRET` invalidates every write key and viewer cookie. Losing `AGENT_GRAPH_ENCRYPTION_KEY` makes every stored log unreadable, and there's no key rotation yet (the format has a version byte for it).
- **Items still to verify:**
  - Hooks on real Windows: the quoted, forward-slash hook command under Claude Code's Windows shell (CI covers our code on Windows, but not Claude Code itself)
  - Claude Code's hook `if` filter for narrowing `Bash` matches
  - Codex and Gemini todo/plan tool names
  - Cursor CLI hook support
  - Codex hooks under `codex exec`
