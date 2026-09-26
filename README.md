# Agent Graph

Records how AI coding sessions relate to each other: which session spawned which agents, who is waiting on whom, what's waiting on *you*, what looks stuck, what work is left, and what they sent each other. It works from provider hooks and writes an append-only log on your machine. Nothing leaves it unless you choose to share.

Supported so far: **Claude Code**. Runs on Windows, macOS and Linux.

See [docs/idea.md](docs/idea.md) for the goal and [docs/design.md](docs/design.md) for how it works.

## Install

Requires Rust (stable).

```bash
cargo install --path .
```

```bash
agent-graph install claude-code
```

This does two things:
- **Recording:** adds hooks to `~/.claude/settings.json`. It keeps everything else in the file and backs it up first. New Claude Code sessions are recorded from then on, including sessions one starts from its shell (`claude -p`, `codex exec`, …), which appear under it. Installed before that was added? Run `install` again to update the hooks.
- **The `/agent-graph` command:** adds a skill you can run in any session. It gives you a picture of the session and a short summary of what needs you, what's stuck and what's in progress, sent to you when the app can show files. `/agent-graph all` covers every recent session, and `/agent-graph share` gives you a live link.

Options:
- `--scope project` or `--scope local` to install for just this project
- `--dry-run` to preview the changes
- `--no-slash-command` for the hooks alone

`agent-graph uninstall claude-code` removes both.

For other coding agents, `install` adds the same command:

| Agent | Command | Written to |
|---|---|---|
| Codex | `agent-graph install codex` | `~/.agents/skills/agent-graph/` (run it as `$agent-graph`) |
| Gemini CLI | `agent-graph install gemini` | `~/.gemini/commands/agent-graph.toml` |
| Cursor | `agent-graph install cursor` | `~/.cursor/skills/agent-graph/` |

Their own sessions aren't recorded yet; only Claude Code has hooks so far.

`agent-graph --help` gives an overview, and `agent-graph <command> --help` explains a command in full, with examples. The same text is on the web at [agentgraph.chofter.com/docs](https://agentgraph.chofter.com/docs).

## Look at your agents

| Command | What it does |
|---|---|
| `agent-graph tail` | Live, full-screen tree in the terminal, redrawn as events arrive. `q` quits, `a` shows older sessions, `--ascii` for plain characters. |
| `agent-graph view --open` | Live web view on port 7777 (it prints a link with a key, `http://127.0.0.1:7777/?key=…`; only that link works). Has a timeline slider for stepping back through a session, saves images, and reopens a Claude Code session in a new terminal window. |
| `agent-graph snapshot` | Saves a phone-sized PNG of the current session and prints its path, ready for an agent to send you. `--out x.svg` for SVG, `--json` for the raw graph. |
| `agent-graph tree` | One-off text tree (`--all` includes older sessions). |
| `agent-graph watch-remote` | Shares the graph live at [agentgraph.chofter.com](https://agentgraph.chofter.com) and prints the link straight away. See below. |
| `agent-graph run -- <command>` | Runs any command as a node in the graph, e.g. an agent without hooks (`agent-graph run -- aider`) or a script that starts several agents, which then appear under it. Exits with the command's exit code. |

```
claude-code:e0000002  search-indexer  [working]  Rebuilding the index schema (+1 pending)  tasks 0/2  waiting on 1 node (0 open tasks), Explore a91d0160 looks stuck
└─ Explore a91d0160  [working] (stale?)  Profile the slow bulk import
```

Every view flags three kinds of trouble:
- **Needs you:** a permission prompt, a question, or a plan waiting for approval.
- **Stale?:** working but silent for 30 minutes (change with `--stale-minutes`).
- **Deadlock:** sessions waiting on each other.

To see all of these at once, open the [example logs](examples/README.md).

## Share

```bash
agent-graph watch-remote
```

This prints a link to a web viewer (the same one as `view`) and keeps it updated as your agents work, until you stop it. You can also paste or upload a log at [agentgraph.chofter.com](https://agentgraph.chofter.com).

| Option | Effect |
|---|---|
| `--password=…` | Viewers must enter it. `--password=` shares without one, even if a default is saved. |
| `--save-default-password` | Saves `--password` for future runs, in a file only you can read. With `--password=`, clears it. |
| `--session=…` | Shares one session instead of all of them. |
| `--url=…` | Shares to another copy of the site, e.g. `http://localhost:3000` while developing it. |

Only the machine that created a shared log can add to it. Every update must carry a key that the site returned when the log was created, and the CLI keeps that key only in memory. The site is in [site/](site/README.md).

Events live in `~/.agent-graph/events/`, one JSON Lines file per session. The format is defined in [schema/event.schema.json](schema/event.schema.json).

| Environment variable | Effect |
|---|---|
| `AGENT_GRAPH_HOME` | Use a different data directory |
| `AGENT_GRAPH_RAW=1` | Also save raw hook payloads to `raw/`, for building adapters |
| `AGENT_GRAPH_CAPTURE_BODIES=1` | Keep message bodies and final agent messages (off by default) |
| `AGENT_GRAPH_NOW` | Pretend it's this time (RFC 3339) when reading logs; used to view the examples |
| `AGENT_GRAPH_AGENT_COMMANDS` | More programs (comma separated) that start an agent session when an agent runs them from its shell (wrappers that start Claude Code, say): any session started from that shell may be the one they started |

A session passes its identity to anything it starts through `AGENT_GRAPH_PARENT` and a W3C `TRACEPARENT` in its shell's environment, so a session started from another links itself under it. When those are missing, it's matched by process instead.

## Develop

```bash
cargo test
```

All the help text lives in [docs/cli-help.json](docs/cli-help.json). `build.rs` compiles it into the binary, and the site's /docs page renders the same file, so edit it there, never in code. A test fails if a command or option has no help text, or if the file describes one that doesn't exist.

The example logs are generated by `tests/examples.rs`; see [examples/README.md](examples/README.md) to regenerate them. The site has its own setup; see [site/README.md](site/README.md).
