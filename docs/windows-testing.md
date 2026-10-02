# Testing on Windows, by hand

CI runs every test on its native Windows runner, but nothing there runs the real Claude Code, logs in to Windows, or looks at a window. This checklist covers what CI can't: agent-graph under the real Claude Code on Windows, and `watch-remote --autostart` across a real login. It's written for **sullyhome**, Chofter CI's Windows x86_64 machine, signed in to Windows as you (the same account the CI runner uses is fine).

Every step happens **on sullyhome**. "PowerShell" means a new Windows PowerShell or PowerShell 7 window from the Start menu, not Git Bash, and not a terminal inside Claude Code. Tick each box as you go. Anything that doesn't happen as described is a finding: note the step, what you saw, and keep the files listed at the end.

## 0. Before you start

- [ ] **Claude Code is installed, natively.** In PowerShell: `claude --version`. If it isn't there, install it (it needs [Git for Windows](https://git-scm.com/download/win), which the CI machine has):

  ```powershell
  irm https://claude.ai/install.ps1 | iex
  ```

- [ ] **No agent-graph from before.** In PowerShell, `Get-Command agent-graph -ErrorAction SilentlyContinue` prints nothing. If it prints a path, run `agent-graph uninstall claude-code` and delete that file first.

## 1. Install agent-graph

The build comes from Chofter CI's own output on this machine. Use the newest passing run's: its ID is in the URL of its page under the repository's Actions tab on GitHub (`…/actions/runs/<ID>`). Run 37002279479 (commit d8c35b9) has everything this checklist tests.

- [ ] In PowerShell, download it to `%USERPROFILE%\.local\bin` and put that folder on your PATH:

  ```powershell
  $run = "37002279479"
  New-Item -ItemType Directory -Force "$env:USERPROFILE\.local\bin" | Out-Null
  Invoke-WebRequest "http://192.168.1.12:4311/runs/$run/dist/agent-graph-x86_64-pc-windows-msvc.exe" -OutFile "$env:USERPROFILE\.local\bin\agent-graph.exe"
  [Environment]::SetEnvironmentVariable("Path", "$env:USERPROFILE\.local\bin;" + [Environment]::GetEnvironmentVariable("Path", "User"), "User")
  ```

- [ ] Close that PowerShell window, open a new one (so it has the new PATH), and run `agent-graph --version`. It prints `agent-graph 0.1.17` or later. (If Windows says it blocked the file, run `Unblock-File "$env:USERPROFILE\.local\bin\agent-graph.exe"`, and note it.)

## 2. Set up the hooks

- [ ] In PowerShell:

  ```powershell
  agent-graph install claude-code
  ```

  It shows the change and asks first. Say yes. When it then offers to share live (autostart), **say no**: section 9 tests that on its own.
- [ ] Look at what it wrote: `notepad "$env:USERPROFILE\.claude\settings.json"`. Each agent-graph hook's command is the full path to `agent-graph.exe`, **with forward slashes, in double quotes**, then `emit --provider claude-code`, e.g. `"C:/Users/shane/.local/bin/agent-graph.exe" emit --provider claude-code`. A backup of the file from before is beside it.
- [ ] `/agent-graph` is installed: `Test-Path "$env:USERPROFILE\.claude\skills\agent-graph\SKILL.md"` prints `True`.

## 3. A session is recorded

Use a scratch folder, so nothing real is touched.

- [ ] In PowerShell, make the folder and start Claude Code in it:

  ```powershell
  New-Item -ItemType Directory -Force "$env:USERPROFILE\ag-test" | Out-Null
  cd "$env:USERPROFILE\ag-test"
  git init
  claude
  ```

- [ ] In Claude Code, send: `Write a file hello.txt that says hi.`
- [ ] In a **second** PowerShell window, run `agent-graph tree`. The session is listed (as `ag-test`, then by the name Claude Code gives it), working while Claude works and idle once it's done.
- [ ] `Test-Path "$env:USERPROFILE\.agent-graph\emit.log"` prints `False`. (If `True`, the hook hit errors: open it, and keep it.)
- [ ] `Get-ChildItem "$env:USERPROFILE\.agent-graph\events"` lists a `claude-code-<id>.jsonl` file.

## 4. Agents, and what needs you

Still in that Claude Code session (window 1), with `agent-graph tail` running in window 2 (`q` quits it):

- [ ] Send: `Use an Explore agent to find every .txt file here.` In `tail`, an Explore agent appears **under** the session while it works, then shows as done.
- [ ] Send: `Run the command "whoami" in the shell.` If Claude Code asks permission, `tail` shows the session as **needs you** (with what it's asking) until you answer.
- [ ] Send: `Make a todo list of three steps for tidying this folder, then do the first.` `tail` shows the task in progress, and the open count.

## 5. A session started from the shell

The hard one: a `claude -p` the session runs in its (Git Bash) shell must link itself under the session, through the environment the SessionStart hook exports.

- [ ] In the session (window 1), send: `Run this in the shell, exactly: claude -p "say hello in five words"`
- [ ] In window 2, `agent-graph tree`: a second Claude Code session appears **under** the first, not beside it.
- [ ] `agent-graph view --open` opens the viewer in your browser. Click the child session: its details say **Linked by: Inherited from its parent's shell**. (If it says *Its processes*, the environment didn't get through but the process tree did: note it. If it isn't under the parent at all, both failed.)

## 6. The slash command

- [ ] In the session (window 1), send `/agent-graph`. Claude replies with a picture of the session and a summary of what needs you, what's stuck and what's in progress.

## 7. The views

- [ ] In window 2, `agent-graph view --open`: the graph is there, live (send another prompt in window 1 and watch it change), and the timeline steps back through the session.
- [ ] Quit Claude Code in window 1 (`/exit`). In the viewer, open the session's details and click **Resume in Claude Code**: a new `cmd` window opens in `ag-test`, running Claude Code on that conversation. Close it. (While a session's still running, the button says **Open a copy in Claude Code** instead.)
- [ ] If the Claude desktop app is installed and has the session, the details offer **Open in** the app, which shows it there.
- [ ] In window 2, `agent-graph snapshot`: it prints a `.png` path. Open it (`start <path>`): a picture of the session, with readable text.

## 8. `run`

- [ ] In window 2: `agent-graph run -- cmd /c echo hi`. It prints `hi`, and `agent-graph tree` shows a run node for it, completed.
- [ ] `agent-graph run -- claude -p "say hi"`: it starts Claude Code (installed as a `.cmd` or `.exe` on PATH), and the session appears under the run.
- [ ] `agent-graph run -- cmd /c exit 3`, then `$LASTEXITCODE` prints `3`.

## 9. Sharing at login (`watch-remote --autostart`)

Needs an account on agentgraph.chofter.com (the one you use on your Mac is fine).

- [ ] In window 2:

  ```powershell
  agent-graph watch-remote --autostart
  ```

  If this computer isn't logged in, your browser opens to log in. It then says it'll share whenever you log in, that **it's in Task Manager's Startup apps**, and where its log is. It returns to the prompt straight away (it doesn't sit waiting).
- [ ] Two `agent-graph.exe` processes are running, one the other's parent:

  ```powershell
  Get-CimInstance Win32_Process -Filter "Name='agent-graph.exe'" | Select-Object ProcessId, ParentProcessId, CommandLine | Format-List
  ```

  One's command line has `--autostarted` (the supervisor), the other `--background` (the one sharing), and its parent is the supervisor. No new window appeared.
- [ ] `notepad "$env:USERPROFILE\.agent-graph\watch-remote.log"` shows it sharing. On your phone or Mac, agentgraph.chofter.com/watch shows sullyhome's sessions.
- [ ] Task Manager (Ctrl+Shift+Esc) → **Startup apps**: there's an entry for agent-graph, enabled.
- [ ] Close window 2 (the PowerShell you ran `--autostart` from). Run the `Get-CimInstance` command again in another window: both processes are still running.
- [ ] **It's started again after a failure.** Stop the `--background` one (its ProcessId from above): `Stop-Process -Id <ProcessId> -Force`. Within about a minute, the log says `watch-remote stopped (…): starting it again in 60 seconds.`, and a new `--background` process is running under the same supervisor.
- [ ] **It starts at login.** Sign out of Windows (Start → your picture → Sign out) and sign back in. A console window may flash briefly; note whether it does, and for how long. The two processes are running again, and the log has a new start.
- [ ] **It stops for good.** In PowerShell:

  ```powershell
  agent-graph watch-remote --no-autostart
  ```

  It says it's stopped. No `agent-graph.exe` processes are left (the `Get-CimInstance` command prints nothing), the Startup apps entry is gone, and `watch-remote.pid` is gone from `%USERPROFILE%\.agent-graph`.

## 10. Codex (if it's installed)

Codex on Windows hasn't been tried at all, so anything here is new.

- [ ] In PowerShell: `agent-graph install codex`, then start `codex` in `ag-test` and give it a small task. (The Codex desktop app doesn't put `codex` on your PATH: use the Codex CLI, `npm install -g @openai/codex`, or the app itself. A task that says the workspace is read-only is Codex's sandbox setting, not agent-graph.)
- [ ] `notepad "$env:USERPROFILE\.codex\hooks.json"`: each agent-graph command starts with `& "` (PowerShell's call operator: Codex runs hooks with PowerShell on Windows, which needs it).
- [ ] `agent-graph tree` shows the Codex session. If it doesn't, check `%USERPROFILE%\.agent-graph\emit.log`, and whether `%USERPROFILE%\.codex\hooks.json` has agent-graph's hooks.

## 11. Clean up

- [ ] In PowerShell:

  ```powershell
  agent-graph uninstall claude-code
  agent-graph uninstall codex
  ```

  `settings.json` no longer mentions agent-graph (open it in Notepad to check), and Claude Code still starts.
- [ ] Leave `agent-graph.exe` in `.local\bin` if you'd like to keep using it, or delete it. `%USERPROFILE%\ag-test` and `%USERPROFILE%\.agent-graph` can be deleted.

## What to send back

For each step that didn't go as described: its number, what you saw (a screenshot helps), and these files from sullyhome, if they exist:

- `%USERPROFILE%\.agent-graph\emit.log`: the hooks' errors
- `%USERPROFILE%\.agent-graph\watch-remote.log`: sharing at login
- `%USERPROFILE%\.claude\settings.json`: the hooks as installed (before step 11)
- the `events\claude-code-*.jsonl` files for the sessions in question
