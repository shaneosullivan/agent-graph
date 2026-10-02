# @chofter/agent-graph

Records how your AI coding sessions relate to each other: which session spawned which agents, who's waiting on whom, what's waiting on *you*, what looks stuck, and what work is left. It works from Claude Code's and Codex's hooks, and keeps an append-only log on your machine. Nothing leaves it unless you choose to share.

For macOS and Linux (ARM and x86_64), with Node 16 or later. Windows is coming.

## Install

```bash
npm install -g @chofter/agent-graph
```

Install it globally, not with `npx`: the hooks run the program from where npm put it, and `npx`'s copy doesn't stay there. npm installs only the program for your machine (about 3.5 MB), from an optional dependency, `@chofter/agent-graph-<platform>`, so don't install with `--omit=optional`.

## Start recording

```bash
agent-graph install claude-code
agent-graph install codex
```

Each adds hooks to that agent's settings (`~/.claude/settings.json`; `~/.codex/hooks.json` and `config.toml`), keeping everything else in the file and backing it up first. New sessions are recorded from then on, including agents and sessions they start from their shell (`claude -p`, `codex exec`, …), which appear under them.

Each also adds a command you can run in a session: `/agent-graph` in Claude Code, `$agent-graph` in Codex. It gives you a picture of the session and a short summary of what needs you, what's stuck and what's in progress. `/agent-graph all` covers every recent session, and `/agent-graph share` gives you a live link.

| Option | Effect |
|---|---|
| `--scope project` or `--scope local` | Install for just this project (Claude Code) |
| `--dry-run` | Show the changes without making them |
| `--no-slash-command` | The hooks alone |

`agent-graph uninstall claude-code` and `agent-graph uninstall codex` remove it all again.

The hooks name the program by its path inside npm's global folder, which upgrades keep. If you switch to another Node version (with nvm, say), which has its own global folder, install @chofter/agent-graph there and run `agent-graph install …` again.

## Look at your agents

| Command | What it does |
|---|---|
| `agent-graph tail` | Live, full-screen tree in the terminal. `q` quits, `a` shows older sessions. |
| `agent-graph view --open` | Live web view on your machine (port 7777), with a timeline for stepping back through a session. |
| `agent-graph snapshot` | Saves a PNG of the current session and prints its path. `--out x.svg` for SVG, `--json` for the raw graph. |
| `agent-graph tree` | One-off text tree (`--all` includes older sessions). |
| `agent-graph watch-remote` | Shares the graph live to your account at [agentgraph.chofter.com](https://agentgraph.chofter.com/watch), where only you can see it. |
| `agent-graph run -- <command>` | Runs any command as a node in the graph, e.g. an agent without hooks (`agent-graph run -- aider`). |

Every view flags what needs you (a permission prompt, a question, a plan to approve), agents that have gone quiet while working, and sessions waiting on each other.

`agent-graph --help` gives an overview, and `agent-graph <command> --help` explains a command in full. The same is at [agentgraph.chofter.com/docs](https://agentgraph.chofter.com/docs).

## Upgrade and remove

```bash
npm update -g @chofter/agent-graph
```

To remove it, run `agent-graph uninstall claude-code` (and `codex`) first, so no hooks are left pointing at it, then `npm uninstall -g @chofter/agent-graph`. Your logs stay in `~/.agent-graph/`.

## License

MIT
