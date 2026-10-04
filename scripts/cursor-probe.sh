#!/bin/sh
# Records what Cursor's hooks are given, for docs/cursor.md's step 1: every
# hook's stdin, and the Cursor variables in its environment, appended to
# ~/.agent-graph-probe/. Nothing else is done, and nothing is sent anywhere.
#
#   scripts/cursor-probe.sh install            hooks for every event in ~/.cursor/hooks.json
#   scripts/cursor-probe.sh install --project  the same in this repository's .cursor/hooks.json
#                                              (for a cloud agent: commit it, with this script)
#   scripts/cursor-probe.sh uninstall          puts back the hooks.json there was before
#   scripts/cursor-probe.sh hook               what each hook runs (reads stdin)
#
# The first hook also writes the names (only) of every variable it gets to
# cursor-env-names.txt.
#
# A hook prints `{}`: Cursor reads a hook's output as JSON. On sessionStart
# it also sets AGENT_GRAPH_PROBE_ENV for the session's commands, to see
# whether a session's `env` reaches them.
set -eu

PROBE="$HOME/.agent-graph-probe"
EVENTS="sessionStart sessionEnd beforeSubmitPrompt preToolUse postToolUse \
postToolUseFailure subagentStart subagentStop beforeShellExecution \
afterShellExecution beforeMCPExecution afterMCPExecution beforeReadFile \
afterFileEdit preCompact stop afterAgentResponse afterAgentThought"

hooks_json() {
  command=$1
  printf '{\n  "version": 1,\n  "hooks": {\n'
  first=1
  for event in $EVENTS; do
    [ $first = 1 ] || printf ',\n'
    first=0
    printf '    "%s": [{"command": "%s", "timeout": 10}]' "$event" "$command"
  done
  printf '\n  }\n}\n'
}

case "${1:-hook}" in
install)
  if [ "${2:-}" = --project ]; then
    root=$(git rev-parse --show-toplevel)
    file="$root/.cursor/hooks.json"
    # Project hooks run from the project's root.
    command="sh scripts/cursor-probe.sh hook"
    [ -f "$root/scripts/cursor-probe.sh" ] || cp "$0" "$root/scripts/cursor-probe.sh"
  else
    file="$HOME/.cursor/hooks.json"
    mkdir -p "$PROBE"
    cp "$0" "$PROBE/cursor-probe.sh"
    command="sh $PROBE/cursor-probe.sh hook"
  fi
  mkdir -p "$(dirname "$file")"
  if [ -f "$file" ] && [ ! -f "$file.before-probe" ]; then
    cp "$file" "$file.before-probe"
  fi
  hooks_json "$command" >"$file"
  echo "Wrote $file. Payloads go to $PROBE/cursor.jsonl."
  ;;
uninstall)
  for file in "$HOME/.cursor/hooks.json" "$(git rev-parse --show-toplevel 2>/dev/null || echo .)/.cursor/hooks.json"; do
    if [ -f "$file.before-probe" ]; then
      mv "$file.before-probe" "$file" && echo "Put back $file."
    elif [ -f "$file" ] && grep -q "cursor-probe.sh hook" "$file"; then
      rm "$file" && echo "Removed $file."
    fi
  done
  ;;
hook)
  mkdir -p "$PROBE"
  at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  # One line per payload: JSON escapes the newlines inside its strings, so
  # any others are only layout.
  payload=$(tr -d '\n\r')
  printf '%s\n' "$payload" >>"$PROBE/cursor.jsonl"
  event=$(printf '%s' "$payload" | sed -n 's/.*"hook_event_name" *: *"\([^"]*\)".*/\1/p')
  # Once: the name (not the value) of every variable a hook gets, for
  # whatever marks where it runs (a cloud VM, say).
  if [ ! -f "$PROBE/cursor-env-names.txt" ]; then
    { echo "--- $at $event: every variable's name"; env | cut -d= -f1 | sort; } >"$PROBE/cursor-env-names.txt"
  fi
  {
    echo "--- $at $event (pid $$, parent $PPID)"
    # Values only for what can't be a secret: just the name for anything
    # that might be a key or token.
    env | grep -E '^(CURSOR|CLAUDE|AGENT_GRAPH|CODEX)[A-Z_]*=' | sort |
      sed -E 's/^([A-Z_]*(KEY|TOKEN|SECRET|PASSWORD|AUTH|SCOPES)[A-Z_]*)=.*/\1=(not recorded)/' || true
  } >>"$PROBE/cursor-env.txt"
  if [ "$event" = sessionStart ]; then
    printf '{"env": {"AGENT_GRAPH_PROBE_ENV": "set by sessionStart"}}\n'
  else
    printf '{}\n'
  fi
  ;;
*)
  echo "usage: $0 install [--project] | uninstall | hook" >&2
  exit 2
  ;;
esac
