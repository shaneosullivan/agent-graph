#!/usr/bin/env bash
# Sets up, and takes down, the Cursor stress test (docs/cursor-stress-test.md).
#
#   scripts/cursor-stress/setup.sh start    the workspace, and the hooks with raw capture
#   scripts/cursor-stress/setup.sh finish   the hooks as they were (without raw capture)
#
# `start` makes ~/cursor-stress, a throwaway git repository for the agents to
# work in (nothing outside it is theirs to touch), with:
#   - mark: `~/cursor-stress/mark "A3 start"` appends a UTC-timestamped line to
#     ~/cursor-stress/steps.log, which is how what the hooks record is matched
#     to the step that caused it;
#   - slow.sh: sleeps for as many seconds as it's given;
#   - .cursor/cli.json: the project's CLI rules (allows ls, git, cat and
#     sleep; denies rm), merged with your own ~/.cursor/cli-config.json;
#   - .cursor/agents/slow-worker.md: a custom subagent that runs in the
#     background;
#   - tools/: drive_cli.py, and `step` and `wait`, which run one of its
#     steps in the background and wait for it (a Cursor shell command times
#     out after 30 s);
#   - RUNBOOK.md: docs/cursor-stress-test.md, for Cursor to follow.
# Then it reinstalls Agent Graph's Cursor hooks with AGENT_GRAPH_RAW=1, so
# every payload is also kept, as Cursor sent it, in ~/.agent-graph/raw/. Those
# hold prompts and outputs: they stay on this computer, and `finish` says
# where to delete them.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
WS="$HOME/cursor-stress"
BIN="$ROOT/target/release/agent-graph"

case "${1:-}" in
start)
  [ -x "$BIN" ] || { echo "Build it first: cargo build --release" >&2; exit 1; }
  if [ -e "$WS" ]; then
    echo "$WS already exists: move it away first (or delete it, if it's an old run's)." >&2
    exit 1
  fi
  mkdir -p "$WS/src" "$WS/docs" "$WS/.cursor/agents"
  cd "$WS"
  git init -q
  printf '# Cursor stress test\n\nA throwaway repository for testing Agent Graph with Cursor.\n' >README.md
  for i in 1 2 3 4 5; do printf 'def f%s():\n    return %s\n' "$i" "$i" >"src/mod$i.py"; done
  printf '# Notes\n\nNothing here yet.\n' >docs/notes.md
  printf '#!/bin/sh\n# Sleeps for $1 seconds (default 20), then says so.\nsleep "${1:-20}" && echo "slept ${1:-20}s"\n' >slow.sh
  chmod +x slow.sh
  cat >.cursor/cli.json <<'EOF'
{
  "permissions": {
    "allow": ["Shell(ls)", "Shell(git)", "Shell(cat)", "Shell(sleep)"],
    "deny": ["Shell(rm)"]
  }
}
EOF
  cat >.cursor/agents/slow-worker.md <<'EOF'
---
name: slow-worker
description: Runs ./slow.sh for a while, then reports. Use when asked for the slow worker.
is_background: true
readonly: true
---
Run `./slow.sh 25` in the workspace, then reply with its output and nothing else.
EOF
  cat >mark <<'EOF'
#!/bin/sh
# Appends a timestamped line to steps.log: `./mark "A3 start"`.
printf '%s %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$*" >>"$(dirname "$0")/steps.log"
echo "marked: $*"
EOF
  chmod +x mark
  printf '.cursor/worktrees/\nsteps.log\nnotes.md\ndrive-logs/\ntools/\nRUNBOOK.md\ncli-config-at-start.txt\n' >.gitignore
  git add -A && git -c user.name=stress -c user.email=stress@example.com commit -qm "Start"
  mkdir -p drive-logs tools
  cp "$ROOT/scripts/cursor-stress/drive_cli.py" "$ROOT/scripts/cursor-stress/step" \
    "$ROOT/scripts/cursor-stress/wait" tools/
  cp "$ROOT/docs/cursor-stress-test.md" RUNBOOK.md
  printf '# Notes from the Cursor stress test\n\n' >notes.md

  "$BIN" install cursor --yes --no-slash-command \
    --command "AGENT_GRAPH_RAW=1 \"$BIN\" emit --provider cursor" >/dev/null
  # What Cursor's CLI is set to, for the record.
  python3 - "$HOME/.cursor/cli-config.json" <<'EOF' >"$WS/cli-config-at-start.txt"
import json, sys
c = json.load(open(sys.argv[1]))
print("approvalMode:", c.get("approvalMode"))
print("sandbox:", c.get("sandbox"))
print("permissions:", c.get("permissions"))
EOF
  "$WS/mark" "SETUP done: hooks with raw capture; $("$BIN" --version); cursor $(~/.local/bin/agent --version 2>/dev/null)"
  echo "Ready: $WS. Hooks reinstalled with raw capture. CLI settings: $WS/cli-config-at-start.txt"
  ;;
finish)
  "$BIN" install cursor --yes --no-slash-command \
    --command "\"$BIN\" emit --provider cursor" >/dev/null
  [ -x "$WS/mark" ] && "$WS/mark" "FINISH: hooks back without raw capture"
  echo "Hooks back as they were. The raw payloads are in ~/.agent-graph/raw/cursor-*.jsonl"
  echo "(they hold prompts and outputs): delete them once they've been looked at."
  ;;
*)
  echo "usage: $0 start | finish" >&2
  exit 2
  ;;
esac
