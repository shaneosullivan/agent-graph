# Codex cloud

**Built (0.1.8):** plan A, `agent-graph install codex --cloud` in the setup script, with the hooks synchronous; `watch-remote` trusts `SSL_CERT_FILE`; the host is called "Codex cloud". The site's Download section has a "Codex cloud" tab (`/#install-codex-cloud`). The rest of this file is how it was worked out.

Recording and sharing live the sessions Codex runs in its cloud (a "cloud task", started from chatgpt.com/codex, the Codex app, or `codex cloud`), as Agent Graph does for Claude Code's cloud (`install claude-code --cloud`). Local Codex support ([codex.md](codex.md)) comes first; this builds on it. Written 2026-10-01, from OpenAI's docs and Codex's source; the unknowns below need one experiment in a real cloud environment before building.

## What's known

From OpenAI's docs on [cloud environments](https://learn.chatgpt.com/docs/environments/cloud-environments), and the [legacy version](https://learn.chatgpt.com/docs/environments/cloud-environment) that's being replaced:

- **A task runs in a VM** from a prepared image ("universal": [openai/codex-universal](https://github.com/openai/codex-universal)), with the repository checked out.
- **Setup.** An environment has an **install script** (dependencies), run while the environment is prepared, with internet access, and a **start skill** that starts services and checks they're ready. A task starts from the prepared filesystem, and repository refreshes don't run the script again. (The legacy environments had a setup script, and a maintenance script for cached containers, up to 12 hours.) The setup's shell is separate from the agent's, so an `export` there doesn't reach the task.
- **Settings.** Environment configuration has **environment variables**, which programs see during setup and tasks, and **network secrets**: a program sees a placeholder, and the network proxy puts the real value in its place, only for HTTPS (port 443) to allowed destinations. (The legacy environments' secrets were for setup scripts only.)
- **Internet during a task** is off unless "Allow Codex to access internet" is on, with an allowlist: Package managers, or Custom domains only, plus "Additional allowed domains". Everything goes through an HTTP(S) proxy.
- **The repository's skills** are available to cloud tasks; your own computer's aren't.

Not documented, and not answerable from Codex's source (which has only the client side of cloud tasks):

1. Whether a cloud task runs **hooks** at all: the repository's `.codex/hooks.json` (which needs the project trusted), or `~/.codex/hooks.json` in the VM (which needs each hook trusted in `~/.codex/config.toml`).
2. **Where Codex's home is** in the VM (`$HOME`, `$CODEX_HOME`), and whether what the install script writes there is still there when a task runs.
3. Whether a **process started by the install script or start skill keeps running** through a task (the start skill is for services, so probably).
4. Whether a task writes **rollout files** (`~/.codex/sessions/…/rollout-*.jsonl`) in the VM as the CLI does.
5. What in the environment says it's **Codex's cloud** (as `CLAUDE_CODE_REMOTE=true` does for Claude Code's): any `CODEX_*` variables, the hostname, the user.
6. Whether a network secret's placeholder is replaced in a **request header** (`Authorization: Bearer …`), which is how `watch-remote` sends the API token.

## Step 1: find out (you, about 15 minutes)

Everything here is done at chatgpt.com/codex (or the Codex app), in a cloud environment for any repository you don't mind a throwaway task in.

1. **Open the environment's configuration** (Environment configuration, for the environment you'll use) and:
   - under internet access, turn on "Allow Codex to access internet", choose Custom domains only, and add `agentgraph.chofter.com`;
   - add an environment variable `AGENT_GRAPH_PROBE=plain`, and a network secret `AGENT_GRAPH_PROBE_SECRET` with any made-up value (not a real token);
   - set the install script to:
     ```bash
     mkdir -p ~/.agent-graph-probe && env | sort > ~/.agent-graph-probe/setup-env.txt
     (while true; do date >> ~/.agent-graph-probe/alive.txt; sleep 5; done) >/dev/null 2>&1 &
     ```
2. **Start a task** in that environment with this prompt, and paste its whole answer back to me:
   > Run each of these shell commands and show me their complete output, without changing anything: `echo HOME=$HOME CODEX_HOME=$CODEX_HOME; id; hostname; env | sort`; `ls -la ~ ~/.codex ~/.agent-graph-probe`; `cat ~/.agent-graph-probe/setup-env.txt`; `tail -3 ~/.agent-graph-probe/alive.txt; date`; `cat ~/.codex/config.toml`; `find ~/.codex -maxdepth 4 | head -50`; `codex --version; which codex`; `ps aux | head -40`; `curl -sS -o /dev/null -w '%{http_code}\n' https://agentgraph.chofter.com/install.sh`; `curl -sS -o /dev/null -w '%{http_code}\n' https://example.com`.

That answers 2 to 5, and says whether the network allowlist works as documented. Question 1 (hooks) and 6 (the secret in a header) need step 2's build, and a second task.

### What step 1 found (2026-10-01)

- **Codex runs in the VM** as `/opt/codex/bin/codex app-server`, with `CODEX_HOME=/opt/codex` (not `~/.codex`, which doesn't exist). `codex` isn't on the PATH. So hooks, trust and rollouts, if any, are under `/opt/codex`.
- **The install script's files last** into the task (`~/.agent-graph-probe/setup-env.txt` was there), but **its processes don't**: the background loop never wrote a line, and nothing of it was running.
- **The install script doesn't see `CODEX_HOME`**: it's only set for the task. Anything the install script writes for Codex must name `/opt/codex` itself.
- **Network secrets are for the install script only**: `AGENT_GRAPH_PROBE_SECRET` had its real value there, and wasn't in the task's environment at all (no placeholder). Plain environment variables are in both.
- **What marks Codex's cloud:** `CODEX_INTERNAL_ORIGINATOR_OVERRIDE=codex_web_agent` (also `CODEX_CI=1`), in the task only.
- **The allowlist works:** `agentgraph.chofter.com` gave 200, `example.com` was refused by the proxy (403). Everything goes through `http://proxy:8080`, with its own CA (`SSL_CERT_FILE`).
- HOME is `/root`, running as root; the repository is in `/workspace/<repo>`.
- **Setup starts from the image each time** the environment is republished: nothing an earlier install script wrote is there.

Then, with `CODEX_HOME=/opt/codex agent-graph install codex --yes` in the install script (0.1.7):

- **Hooks run.** Codex (`codex-cli 0.144.0-alpha.4`, as `app-server`, `originator: codex_web_agent`, `source: vscode`) read `/opt/codex/hooks.json`, honoured our trust in `/opt/codex/config.toml`, and ran `SessionStart`: `session.started` was recorded, with the rollout's path.
- **Nothing else was recorded**: no `UserPromptSubmit`, no tool events, no `Stop`, over a task that ran shell commands. The cloud's tools aren't the CLI's: its rollout has `custom_tool_call`s named `exec` (Codex's "code mode", run by `codex-code-mode-host`), which our matcher (`Bash|apply_patch|…`) doesn't name. The other events are all `async`, and may not be run, or not waited for.
- **Rollouts are written**, in `/opt/codex/sessions/YYYY/MM/DD/`.

Then, with every event hooked by hand, synchronous and with no matcher, logging each payload:

- **Every event came**, and is recorded: `SessionStart`, `UserPromptSubmit` (with the prompt and `turn_id`), and `PreToolUse`/`PostToolUse` for `update_plan` and `Bash` (`tool_input.command`, `tool_response`), each with a `tool_use_id` like `exec-<uuid>`. Hooks see the CLI's tool names, not code mode's `exec`, so our matcher is right as it is.
- **So it was `async`**: the cloud doesn't run async hooks (or doesn't wait for them). In the cloud, every hook must be synchronous. Each `emit` took well under 0.1s.
- Payloads also carry `model` (`gpt-5.6-sol`) and `permission_mode: bypassPermissions`, so there are no approvals to wait on.
- **TLS goes through a man-in-the-middle proxy** (`envoy-mitmproxy-ca-cert.crt`, named by `SSL_CERT_FILE` and the other `*_CA*` variables). `watch-remote` uses ureq with rustls and its own bundled roots, so it would likely refuse the proxy's certificate: it has to trust `SSL_CERT_FILE` too. Not yet tried.

## Step 2: build, by what step 1 found

Two ways in, in order of preference. Both install `agent-graph` in the **install script** (it has internet; the task needn't reach anything but `agentgraph.chofter.com`), and share with `watch-remote`, logged in by `AGENT_GRAPH_TOKEN` from the environment's settings, as Claude Code's cloud does.

### A. Hooks, if cloud tasks run them

`agent-graph install codex --cloud`, run **in the install script** (not committed to the repository):

- writes the hooks to the VM's own `~/.codex/hooks.json` and trusts them in its `~/.codex/config.toml`, exactly as `install codex` does on a computer. User-level hooks don't need the project trusted, and nothing is committed;
- starts nothing itself: the `SessionStart` hook starts `watch-remote --background` (as Claude Code's cloud hook does), so sharing starts with the task, whatever the setup's processes do;
- tells the share it's "Codex cloud" (a file in `~/.agent-graph`, since no variable says so).

What you'd set up, all in the environment's configuration at chatgpt.com/codex:

- install script: `curl -fsSL https://agentgraph.chofter.com/install.sh | sh && ~/.local/bin/agent-graph install codex --cloud`;
- environment variable (or network secret, if step 2's check shows it works in a header): `AGENT_GRAPH_TOKEN=<an API token from your account page>`;
- internet access: Custom domains only, with `agentgraph.chofter.com`.

Checking question 1 is then a task in that environment: if the task shows up at `/watch`, hooks run. If not, B.

### B. Reading Codex's own record of the task, if hooks don't run

Codex writes everything a session does to its rollout file (question 4): each tool call (`update_plan`, `spawn_agent`, `exec_command`), each turn's start and end, and its subagents' own rollouts, which name their parents. A **rollout adapter** reads them as the hooks would have:

- `agent-graph watch-codex-rollouts` (a mode of `watch-remote`, say) follows `~/.codex/sessions/`, turns new lines into the same events the hook adapter makes (sharing its code: `src/adapter/codex.rs` would get a second front end), and shares them;
- started by the **start skill** (if step 1 shows its processes last through a task), or by `AGENTS.md` asking Codex to start it first (weaker: it depends on the model);
- the rollout also has what hooks don't: when an approval is answered, and the session's name once it has one.

B needs more building (a second front end on the adapter, tailing files that are rotated and compressed), so it's only if A can't work.

### Either way

- **The site:** a "Codex cloud" tab in the Download section, beside "Claude Code cloud", saying where each step is done (which page of the environment's configuration, what goes in each field), and `/#install-codex-cloud`. The account page's new-token note links to both.
- **The host label:** "Codex cloud", as "Claude Code cloud" is for Claude Code's (`account::host_name`).
- **Tests:** the install (the hooks go in the VM's user files, trusted, with the share hook), the host label, and for B, the rollout adapter against rollouts captured from a real task (and from the local CLI, which writes the same format).
- **Docs:** `docs/cli-help.json` (`--cloud` for Codex), the README, the FAQ (which says Codex's cloud agents aren't supported yet).

## Risks

- **Nothing may run.** If cloud tasks run no hooks, no start-skill process lasts through a task, and there are no rollouts in the VM, Codex's cloud can't be recorded from inside, and the remaining route is OpenAI's own task API, which isn't public. Step 1 finds this out before anything's built.
- **The token.** As an environment variable, the task (and so the model) can read it. It can only share to your account, and you can delete it on the account page, but a network secret, where the model sees only a placeholder, is better if it works in a header.
- **The environment changes under us.** OpenAI is replacing its cloud environments (the "legacy" ones had setup and maintenance scripts; the new ones have an install script and a start skill), so the steps on the site should name fields as the UI does, and be checked against it.
