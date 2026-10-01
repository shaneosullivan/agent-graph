#!/usr/bin/env bash
# Records a real Codex CLI session end to end, with no OpenAI account: Codex
# talks to a stand-in model (scripts/codex-mock-model.mjs) that plays a
# scripted session with a plan, a subagent it waits for, and a command that
# starts another agent session from Codex's shell. Agent Graph's hooks are
# put in with `agent-graph install codex`, and Codex runs them only because
# the install trusted them (no --dangerously-bypass-hook-trust). Then the
# graph is checked.
#
#   scripts/codex-e2e.sh [path/to/codex]
#
# Needs node, and Codex: the one given, or on PATH, or else it's installed
# from npm into a temporary folder. Nothing outside temporary folders is
# touched: CODEX_HOME, AGENT_GRAPH_HOME and HOME all point at one.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TMP="$(mktemp -d)"
trap 'kill "${MOCK_PID:-}" 2>/dev/null || true; rm -rf "$TMP"' EXIT

cargo build --quiet --manifest-path "$ROOT/Cargo.toml"
AG="$ROOT/target/debug/agent-graph"

CODEX="${1:-$(command -v codex || true)}"
if [ -z "$CODEX" ]; then
  echo "Installing Codex from npm into $TMP/npm…"
  npm install --silent --prefix "$TMP/npm" @openai/codex >/dev/null
  CODEX="$TMP/npm/node_modules/.bin/codex"
fi
export HOME="$TMP/home" CODEX_HOME="$TMP/home/.codex" AGENT_GRAPH_HOME="$TMP/agent-graph"
export AGENT_GRAPH_AGENT_COMMANDS=fakeagent
unset AGENT_GRAPH_PARENT AGENT_GRAPH_PARENT_CODEX TRACEPARENT CODEX_THREAD_ID CODEX_SESSION_ID
mkdir -p "$CODEX_HOME" "$TMP/project/bin"
# (Only now: Codex makes itself a home even to say its version.)
echo "Codex: $("$CODEX" --version)"
PORT=$((20000 + RANDOM % 20000))
cat > "$CODEX_HOME/config.toml" <<TOML
# The stand-in model.
model = "gpt-5.5"
model_provider = "mock"
approval_policy = "never"
sandbox_mode = "danger-full-access"

[model_providers.mock]
name = "mock"
base_url = "http://127.0.0.1:$PORT/v1"
env_key = "PATH"
wire_api = "responses"
request_max_retries = 0
stream_max_retries = 0
TOML
# (Not the plan tool: `install codex` turns it on.)

# "Another agent", started from Codex's shell: records a session of its own,
# as an agent's hooks would, which should link itself to Codex's.
cat > "$TMP/project/bin/fakeagent" <<SH
#!/bin/sh
echo '{"session_id":"child-session","hook_event_name":"SessionStart","cwd":"$TMP/project","source":"startup"}' | "$AG" emit --provider claude-code
echo '{"session_id":"child-session","hook_event_name":"SessionEnd","reason":"exit"}' | "$AG" emit --provider claude-code
SH
chmod +x "$TMP/project/bin/fakeagent"

cat > "$TMP/scenario.json" <<'JSON'
{
  "main": [
    [{"call": "update_plan", "args": {"plan": [{"step": "Look around", "status": "in_progress"}, {"step": "Delegate the check", "status": "pending"}]}}],
    [{"call": "spawn_agent", "namespace": "multi_agent_v1", "args": {"message": "SUBTASK: check the files", "agent_type": "explorer"}}],
    [{"call": "wait_agent", "namespace": "multi_agent_v1", "args": {"targets": ["$AGENT"], "timeout_ms": 30000}}],
    [{"call": "exec_command", "args": {"cmd": "fakeagent run 'review it'"}}],
    [{"call": "update_plan", "args": {"plan": [{"step": "Look around", "status": "completed"}, {"step": "Delegate the check", "status": "completed"}]}}],
    [{"say": "All done."}]
  ],
  "SUBTASK": [
    [{"call": "exec_command", "args": {"cmd": "echo checking"}}],
    [{"say": "Checked: all fine."}]
  ]
}
JSON
node "$ROOT/scripts/codex-mock-model.mjs" "$TMP/scenario.json" "$PORT" "$TMP/requests.jsonl" >/dev/null &
MOCK_PID=$!
sleep 1

(cd "$TMP/project" && git init -q && "$AG" install codex --yes --no-slash-command >/dev/null)
(cd "$TMP/project" && PATH="$TMP/project/bin:$PATH" "$CODEX" exec "Plan and delegate" </dev/null >"$TMP/codex.log" 2>&1) || {
  cat "$TMP/codex.log"
  exit 1
}
sleep 1 # (the last hooks run in the background)
"$AG" tree --all

# The graph, checked.
node --input-type=module - "$AGENT_GRAPH_HOME/events" <<'JS'
import fs from 'node:fs';
import path from 'node:path';
const dir = process.argv[2];
const events = fs.readdirSync(dir).flatMap((f) =>
  fs.readFileSync(path.join(dir, f), 'utf8').trim().split('\n').map((l) => JSON.parse(l)));
const of = (type) => events.filter((e) => e.type === type);
const fail = (why) => {
  console.error(`FAILED: ${why}`);
  process.exit(1);
};
const session = of('session.started').find((e) => e.node.startsWith('codex:'));
if (!session) fail('no Codex session');
const plans = of('tasks.updated').filter((e) => e.node === session.node);
if (plans.length !== 2 || plans[1].data.items.some((i) => i.status !== 'completed')) fail('the plan');
const spawned = of('agent.spawned')[0];
if (!spawned || spawned.parent !== session.node || spawned.data.agent_type !== 'explorer') fail('the subagent');
const returned = of('spawn.returned').find((e) => e.data.child);
if (returned?.data.child !== spawned.node) fail('the spawn named its child');
if (!of('agent.finished').some((e) => e.node === spawned.node)) fail('the subagent finished');
if (!of('status').some((e) => e.node === spawned.node && e.data.title)) fail("the subagent's nickname");
const waits = of('wait.started');
if (waits.length !== 1 || waits[0].data.on !== spawned.node) fail('the wait on the subagent');
if (!of('wait.ended').some((e) => e.data.wait_id === waits[0].data.wait_id)) fail('the wait ended');
if (!of('spawn.requested').some((e) => e.data.kind === 'session' && e.data.agent_type === 'fakeagent')) fail('the shell launch');
const child = of('session.started').find((e) => e.node === 'claude-code:child-session');
if (child?.parent !== session.node) fail(`the session started from Codex's shell links to it (${child?.parent})`);
if (!of('status').some((e) => e.node === session.node && e.data.state === 'idle')) fail('the turn ended');
if (!of('session.ended').some((e) => e.node === session.node)) fail('the session ended');
console.log(`OK: ${events.length} events, all as expected.`);
JS
