# Cursor stress test: a runbook for Cursor

This is a runbook for **Cursor's agent** to carry out, in a chat in Cursor's app, with Shane at the keyboard. It stress-tests Agent Graph's Cursor support, which records Cursor's chats through Cursor's hooks: [cursor.md](cursor.md) says what it records and why. Meanwhile a Claude Code session watches what Agent Graph records, step by step, and decides what's right.

The test is you: your own commands, subagents, questions and approvals, and the Cursor CLI chats you start and drive from your shell. It takes about two hours.

## Before the chat (Shane, in Terminal, 3 minutes)

1. In the agent-graph repository:
   ```bash
   cargo build --release && scripts/cursor-stress/setup.sh start
   ```
   That makes `~/cursor-stress`, a throwaway git repository, with this runbook in it as `RUNBOOK.md`. It also reinstalls Agent Graph's Cursor hooks so every payload is also kept as Cursor sent it.
2. In Cursor: File → Open Folder → `~/cursor-stress`.
3. Cursor Settings → Agents → Run Mode: **Run Everything**. If there's no such option, choose **Auto-Review (with Sandbox)** and expect to approve some of the runner's commands. Part B changes it as it goes.
4. A new chat, in **Agent** mode, with this prompt:
   > Read RUNBOOK.md in this workspace and carry it out, from "Rules" to the end, one step at a time. You're the runner it talks about.

## Rules for you, the runner

**Your job is to cause things, carefully and on the record, not to judge them.** The watching session reads what Agent Graph recorded and decides what's right.

1. **Mark every step, before and after**, by running `./mark "<id> start"` and `./mark "<id> end"` (for example `./mark "A2 start"`). Mark anything notable as it happens too (`./mark "B2 Shane approved"`). Marks line up what the hooks recorded with the step that caused it, so a step that isn't marked is wasted. `tools/step` marks its own steps.
2. **Keep notes** in `notes.md`, as you go, under each step's id. Include:
   - what Cursor showed: the exact wording of an approval prompt, and which keys or buttons answered it;
   - anything that didn't go as the step says, and what you did instead;
   - times, where a step asks for them.
3. **Stay in this workspace** (`~/cursor-stress`). Don't touch anything else on this Mac, except where a step says to.
4. **Don't fix or retry around a failure.** Note it, mark it, and go on. A failure is a finding.
5. **Your shell commands time out after about 30 seconds.** So a long step runs in the background with `tools/step`, and you then run `tools/wait <id>` until it says `done`. Each wait takes up to 25 seconds, and between waits you can do nothing else for that step. Run one `tools/step` at a time, unless a step says otherwise.
6. **When a step needs Shane** (to approve, refuse, answer, change a setting, or press something), say exactly what he should do and when, in your reply, before you do what makes Cursor ask. Mark what he did once he's done it. When you need him to do something before you go on, end your turn and wait for his reply.
7. **Steps that say "End your turn"** test what Cursor's hooks send when a turn ends. Do exactly that.
8. **Tell Shane when each part is done**, so he can tell the watching session to look at it.

`tools/step <id> <options>` runs `tools/drive_cli.py`: it starts the Cursor CLI (`agent`) in a pseudo-terminal with a prompt, answers what it asks after a delay (`--rule 'REGEX=>SECONDS=>KEYS'`, in order), types follow-ups (`--say 'SECONDS=>TEXT'`), and quits once the screen has been still for a while (`--quiet SECONDS`). Run `python3 tools/drive_cli.py --help` for the details. The CLI chats it starts are linked under your chat in Agent Graph, since they're started from your shell. That's intended, and part of the test. Their screens are logged in `drive-logs/<id>.log`.

## Setup (2 minutes)

1. `./mark "SETUP start"`.
2. Run `agent status`, and note who's signed in. If it says no one is, stop and tell Shane.
3. Read `cli-config-at-start.txt` and copy it into the notes. `approvalMode` should be `allowlist`. If it isn't, note it, and go on.
4. Check that `~/.agent-graph/raw/` now has a `cursor-*.jsonl` with today's events (`ls -la ~/.agent-graph/raw/`). Your own chat's hooks write there, so it should. If it doesn't, note it.
5. `./mark "SETUP end"`.

## Part A: Cursor's CLI, driven from your shell (about 60 minutes)

### A1. What the CLI says when it asks (5 minutes)

**Checks:** what the CLI's approval prompt says, and which keys answer it.

1. A print run:
   ```bash
   ./mark "A1a start"; agent -p --trust "List the files here with ls, then say how many there are."; ./mark "A1a end"
   ```
2. An interactive run that will ask, since `date` isn't on the project's allowlist. Don't answer it (no rules), so it times out:
   ```bash
   tools/step A1b --arg=--trust --prompt "Run the shell command: date" --quiet 40 --timeout 60
   ```
   Then `tools/wait A1b` until it's done, and read `drive-logs/A1b.log`.
3. In the notes, write the wording that shows the CLI is asking, and the choices it offers, with the keys for each. **Every later step's rules use these.** Below:
   - `ASK` stands for a regular expression matching that wording (for example `Run this command`);
   - `YES` for the keys that approve once (for example `{enter}`, or `y`);
   - `NO` for the keys that refuse (for example `{esc}`, or `n`).

   Write what you chose for each at the top of the notes, and use them in every rule below. If the CLI asked something else first (to trust the workspace, say), note it, and put a rule for it in front of the others.

### A2. The CLI's allowlist (8 minutes)

**Checks:** a command the CLI's rules don't allow is shown as "Waiting for your approval" at once; one they allow records nothing; one they deny is refused without asking. Your `~/.cursor/cli-config.json` rules and this project's `.cursor/cli.json` (ls, git, cat and sleep allowed; rm denied) are merged.

For each, `tools/step` it, then `tools/wait` it until it's done.

1. Allowed, so nothing to approve, and slow:
   ```bash
   tools/step A2a --arg=--trust --prompt "Run exactly this shell command and nothing else: sleep 15" --quiet 20
   ```
2. Not allowed, approved after 20 seconds:
   ```bash
   tools/step A2b --arg=--trust --prompt "Run exactly this shell command and nothing else: date" --rule 'ASK=>20=>YES' --quiet 20
   ```
3. Not allowed, refused after 15 seconds:
   ```bash
   tools/step A2c --arg=--trust --prompt "Run exactly this shell command and nothing else: whoami" --rule 'ASK=>15=>NO' --quiet 20
   ```
4. Denied: it should be refused without asking. Note whether it asked.
   ```bash
   tools/step A2d --arg=--trust --prompt "Run exactly this shell command and nothing else: rm -f docs/does-not-exist.md" --quiet 20 --timeout 90
   ```
5. A chain, half allowed. Note whether it asked:
   ```bash
   tools/step A2e --arg=--trust --prompt "Run exactly this one shell command, as it is, and nothing else: ls && date" --rule 'ASK=>20=>YES' --quiet 20 --timeout 120
   ```

### A3. Flags that change how the CLI asks (10 minutes)

**Checks:** the riskiest assumption. Agent Graph reads the CLI's approval mode from its config file, but `--force`, `--auto-review` and `--sandbox` change it for one run, and hooks can't see a run's flags. So a command that runs without asking under `--force` is expected to show as "Waiting for your approval" until it ends: a false positive this step measures.

1. Run everything, slowly:
   ```bash
   tools/step A3a --arg=--trust --arg=--force --prompt "Run exactly this shell command and nothing else: ./slow.sh 20" --quiet 25
   ```
2. Auto-review. Note whether it asked:
   ```bash
   tools/step A3b --arg=--trust --arg=--auto-review --prompt "Run exactly this shell command and nothing else: ./slow.sh 10" --rule 'ASK=>15=>YES' --quiet 20 --timeout 120
   ```
3. The sandbox on. A command run in the sandbox should record nothing. Note whether it asked:
   ```bash
   tools/step A3c --arg=--trust --arg=--sandbox --arg=enabled --prompt "Run exactly this shell command and nothing else: ./slow.sh 10" --rule 'ASK=>15=>YES' --quiet 20 --timeout 120
   ```
4. The sandbox on, with a command that needs the network. Note whether it asked:
   ```bash
   tools/step A3d --arg=--trust --arg=--sandbox --arg=enabled --prompt "Run exactly this shell command and nothing else: curl -sI https://example.com" --rule 'ASK=>20=>YES' --quiet 20 --timeout 120
   ```
5. `--force`, in a print run:
   ```bash
   ./mark "A3e start"; agent -p --trust --force "Run exactly this shell command and nothing else: date"; ./mark "A3e end"
   ```

### A4. Replies ending on a question, in the CLI (8 minutes)

**Checks:** a turn whose last reply ends on a question shows "Asks you a question", and one that doesn't is idle. It also checks what the CLI's question tool sends to hooks: the app's sends nothing.

1. Ends on a question, answered 25 seconds later:
   ```bash
   tools/step A4a --arg=--trust --prompt "Without running anything, ask me which of the files in src I'd like summarised. End your reply with that question, and wait for my answer." --say '40=>mod3.py' --quiet 25 --timeout 150
   ```
2. A question in the middle, not at the end:
   ```bash
   tools/step A4b --arg=--trust --prompt "Without running anything, reply with exactly these two sentences: Should we rename it? No, it's fine as it is." --quiet 20
   ```
3. A question inside formatting:
   ```bash
   tools/step A4c --arg=--trust --prompt "Without running anything, reply with exactly this and nothing else: **Shall I go on?**" --quiet 20
   ```
4. A code block whose last line has a question mark. It isn't a question to you, but it's expected to show as one: a known false positive.
   ```bash
   tools/step A4d --arg=--trust --prompt "Without running anything, reply with only a fenced code block holding this one line of JavaScript: const ok = x ? 1 : 2; // why?" --quiet 20
   ```
5. The question tool, if the CLI has one. The rule answers 30 seconds after the choices appear, with the first. Note whether a question UI appeared at all; if `{enter}` doesn't answer it, note what would.
   ```bash
   tools/step A4e --arg=--trust --prompt "Use your tool for asking me a multiple-choice question (not plain text) to ask whether I prefer tabs or spaces. Then tell me what I chose." --rule 'tabs.*spaces|spaces.*tabs=>30=>{enter}' --quiet 25 --timeout 150
   ```

### A5. Plan and Ask modes, in the CLI (6 minutes)

**Checks:** whether the CLI's hooks say which mode a chat is in, and what a plan waiting for you fires.

1. Plan mode. Nothing answers for 30 seconds after the plan appears. In the notes, say what the screen offered for the plan.
   ```bash
   tools/step A5a --arg=--trust --arg=--plan --prompt "Plan how to add a CHANGELOG.md to this repository. Don't ask me anything first." --quiet 30 --timeout 180
   ```
2. Ask mode:
   ```bash
   tools/step A5b --arg=--trust --arg=--mode --arg=ask --prompt "What does slow.sh do?" --quiet 20
   ```

### A6. Subagents, in the CLI (10 minutes)

**Checks:** each subagent is paired with the call that started it, even when several start at once. A background subagent, whose stop Cursor is said not to send, is ended when its chat's turn or chat ends. A subagent's own commands are left out by Agent Graph (their hooks don't name the chat), which leaves something unseen: a subagent waiting for approval.

1. Three in parallel:
   ```bash
   tools/step A6a --arg=--trust --arg=--force --prompt "Use three subagents in parallel, at once: one counts the files in src, one reads README.md and gives its title, one lists docs. Then summarise in one line." --quiet 25 --timeout 240
   ```
2. The background `slow-worker` subagent, with the chat going on meanwhile:
   ```bash
   tools/step A6b --arg=--trust --arg=--force --prompt "Start the slow-worker subagent in the background. While it runs, tell me how many lines README.md has. Then wait for slow-worker and tell me what it said." --quiet 40 --timeout 300
   ```
3. A subagent whose command needs approval (no `--force`). In the notes, say whose question the screen showed it as:
   ```bash
   tools/step A6c --arg=--trust --prompt "Use one subagent to run the shell command date and report its output. Don't run anything yourself." --rule 'ASK=>20=>YES' --quiet 25 --timeout 240
   ```
4. A subagent told to start a subagent of its own (Cursor is said to allow one level). Note what happened:
   ```bash
   tools/step A6d --arg=--trust --arg=--force --prompt "Use a subagent, and tell it to use a subagent of its own to count the files in src. Report what came back." --quiet 30 --timeout 240
   ```

### A7. Stopping a CLI turn (6 minutes)

**Checks:** a turn stopped part-way ends idle, and a foreground subagent it started is canceled. A chat whose process is killed has nothing to end it (expected): it shows as working until the viewer calls it stale.

1. Stopped 12 seconds in, while its subagents work. If `{esc}` doesn't stop it, run it again as `A7a2` with `{ctrl-c}`, and note which worked.
   ```bash
   tools/step A7a --arg=--trust --arg=--force --prompt "Use two subagents in parallel, each running ./slow.sh 40, then report." --rule '.=>12=>{esc}' --quiet 20 --timeout 120
   ```
2. Killed part-way:
   ```bash
   ./mark "A7b start"; (agent -p --trust --force "Run ./slow.sh 60 and report." >drive-logs/A7b.log 2>&1 &) ; sleep 15; pgrep -fl "slow.sh 60"
   ```
   Then kill the `agent` process running it with `kill -9 <pid>` (only that one: check its command line says `slow.sh 60`), and `./mark "A7b killed"`. Note the pid you killed.

### A8. Resuming a CLI chat (8 minutes)

**Checks:** a resumed chat keeps its conversation id, so it's the same session in the graph, and what `sessionStart` says when it resumes.

1. Start a chat, and get its id from the JSON (note which field holds it):
   ```bash
   ./mark "A8a start"; agent -p --trust --output-format json "Remember the word lantern. Reply with just OK." | tee -a notes.md; ./mark "A8a end"
   ```
2. Resume it by id, in print mode:
   ```bash
   ./mark "A8b start"; agent -p --trust --resume <id> "What word did I ask you to remember? Reply with just the word."; ./mark "A8b end"
   ```
3. Resume it interactively:
   ```bash
   tools/step A8c --arg=--trust --arg=--resume --arg=<id> --prompt "Spell that word backwards." --quiet 20
   ```
4. `agent resume` (the latest chat) and `--continue`. If `agent resume` takes no prompt, drop `--prompt` and use `--say '5=>Reply with just: done'`, and note it.
   ```bash
   tools/step A8d --arg=resume --prompt "Reply with just: done" --quiet 20
   ```
   Then, once that's done:
   ```bash
   tools/step A8e --arg=--trust --arg=--continue --prompt "Reply with just: done again" --quiet 20
   ```

### A9. Sessions a chat starts (8 minutes)

**Checks:** a session started from a Cursor chat's shell links to that chat (by `CURSOR_CONVERSATION_ID`), whatever its agent: Claude Code, Codex, or Cursor's CLI. One started from a subagent's shell is expected to link to nothing, because its conversation isn't named anywhere else.

Shane's Claude Code hooks are Homebrew's 0.1.7 for now, which make a stray `claude-code:<chat id>` session of every Cursor chat, yours included. That's expected, and the watching session knows it.

1. Claude Code, from a CLI chat:
   ```bash
   tools/step A9a --arg=--trust --arg=--force --prompt "Run exactly this shell command and nothing else: claude -p 'Reply with just: hello from claude'" --quiet 25 --timeout 180
   ```
2. Codex, from a CLI chat (it's inside the ChatGPT app):
   ```bash
   tools/step A9b --arg=--trust --arg=--force --prompt "Run exactly this shell command and nothing else: /Applications/ChatGPT.app/Contents/Resources/codex-cli/bin/codex exec --skip-git-repo-check 'Reply with just: hello from codex'" --quiet 25 --timeout 240
   ```
3. Another Cursor CLI chat, from a CLI chat (three levels, counting yours):
   ```bash
   tools/step A9c --arg=--trust --arg=--force --prompt "Run exactly this shell command and nothing else: agent -p --trust 'Reply with just: hello from the inner agent'" --quiet 25 --timeout 240
   ```
4. Claude Code, started by Claude Code, started from a CLI chat:
   ```bash
   tools/step A9d --arg=--trust --arg=--force --prompt "Run exactly this shell command and nothing else: claude -p --allowedTools Bash \"Run this with Bash and report its output: claude -p 'Reply with just: hello from the inner claude'\"" --quiet 30 --timeout 300
   ```
5. Claude Code, from a CLI chat's subagent:
   ```bash
   tools/step A9e --arg=--trust --arg=--force --prompt "Use one subagent to run this shell command and report its output: claude -p 'Reply with just: hello from the subagent'. Don't run it yourself." --quiet 30 --timeout 240
   ```
6. Claude Code, from **your own** shell (this app chat):
   ```bash
   ./mark "A9f start"; claude -p 'Reply with just: hello from the runner'; ./mark "A9f end"
   ```

### A10. A worktree (3 minutes)

**Checks:** a chat in a worktree is its own session, in the worktree's folder.

```bash
./mark "A10 start"; agent -p --trust --force --worktree stress-wt "Run pwd and report it."; ./mark "A10 end"
```

If it takes longer than your shell allows, run it in the background (`… >drive-logs/A10.log 2>&1 &`), and wait for it with `pgrep -fl stress-wt`.

### A11. Load (6 minutes)

**Checks:** nothing is lost or garbled when many hooks run at once, and hooks stay quick. Six print runs at once, each running four commands and two subagents, all in the background:

```bash
./mark "A11 start"; for i in 1 2 3 4 5 6; do (agent -p --trust --force "Run these one at a time: ls, git status, cat README.md, ./slow.sh 3. Then use two subagents in parallel to count the files in src and in docs. Then reply with just: run $i done" >drive-logs/A11-$i.log 2>&1 &); done; echo started
```

Then check every 20 seconds or so with `grep -l "done" drive-logs/A11-*.log | wc -l` until it's 6 (or 4 minutes have passed: note which). Then `./mark "A11 end"`, note how long it took, and run:

```bash
tail -20 ~/.agent-graph/emit.log
```

Note any lines in it from during the run.

**Tell Shane that Part A is done**, and end your turn. Go on with Part B when he says to.

## Part B: you, in the app, with Shane (about 60 minutes)

Here you test yourself: your own commands, subagents, questions and turns, as Cursor's app shows them. Shane changes settings and answers when you ask him to.

### B1. Your commands, in Run Everything mode (4 minutes)

**Checks:** commands that run without asking. One that runs outside the sandbox and takes a while is expected to show as "May be waiting for your approval" after 10 seconds: a known false positive this step measures.

1. `./mark "B1 start"`, then run `./slow.sh 20` (as a background command if your shell's timeout needs it), then `./mark "B1 end"`.

### B2. Your commands, in Allowlist mode (10 minutes)

Ask Shane to set Run Mode to **Allowlist**, and to add `./mark` and `tools/wait` to the Command Allowlist (and nothing else: the suggested `+ date`-style chips aren't on the list unless clicked). End your turn and wait for him to say it's done.

1. Tell Shane: "I'm about to run `date`. When Cursor asks, wait 30 seconds, then approve it once." Then `./mark "B2a start"`, run `date`, and `./mark "B2a end"`.
2. Tell Shane: "I'm about to run `whoami`. When Cursor asks, wait 20 seconds, then refuse it." Then mark, run it, mark. Note what the refusal looked like to you.
3. Ask Shane to add `sleep` to the allowlist, and wait for him. Then mark, run `sleep 25` (in the background, if your shell needs it; it shouldn't ask), mark. Then ask Shane to take `sleep` off the list again.
4. Tell Shane: "I'm about to use a subagent to run `date`. When Cursor asks, wait 20 seconds, then approve it." Then mark, start one subagent to run `date` and report its output (you don't run it), mark. Note where Cursor showed the approval: your chat, or the subagent's.

Ask Shane to set Run Mode back to **Run Everything**, and wait for him.

### B3. Your subagents (8 minutes)

**Checks:** as A6, in the app.

1. Mark, then start three subagents in parallel, at once: one counts the files in src, one reads README.md and gives its title, one lists docs. Then summarise in one line, and mark.
2. Mark, then start the `slow-worker` subagent in the background. While it runs, count the lines in README.md. Then wait for it, report what it said, and mark.
3. Mark, then start a subagent and tell it to start a subagent of its own to count the files in src. Report what came back, and mark.
4. Mark, then start one subagent to run `claude -p 'Reply with just: hello from your subagent'` and report the output, and mark.

### B4. Your turns ending on a question (8 minutes)

**Checks:** a turn ending on a question shows "Asks you a question" until Shane answers, and Cursor's question tool still sends nothing to hooks.

1. Mark `B4a start`. Then, without running anything else, end your turn with exactly this question: "Which file in src should I summarise?" Shane waits **30 seconds** before answering. When he does, mark `B4a end`, summarise the file he names in one line, and go on.
2. Mark `B4b start`. Then use your multiple-choice question tool (not plain text) to ask Shane whether he prefers tabs or spaces. Tell him first to wait **30 seconds** after it appears before answering. When he has, mark `B4b end`, say what he chose, and go on.
3. Mark `B4c start`. Then end your turn with a reply whose last line is a sentence that isn't a question, such as "That's B4c done." When Shane says to go on, mark `B4c end`.
4. Mark `B4d start`. Then end your turn with a reply whose last line is a question in bold: "**Shall I go on to B5?**" Shane waits **20 seconds**, then says yes. Mark `B4d end`.

### B5. Plan mode, and asking to switch modes (8 minutes)

**Checks:** whether a chat in Plan mode says so in its hooks (`composer_mode`), what a plan waiting for Shane fires, and the question Cursor asks before switching modes.

1. Mark `B5a start`. Then ask to switch to Plan mode, to plan a CONTRIBUTING.md (use your tool for switching modes, if you have one). Tell Shane first: when Cursor asks to switch, wait **10 seconds** (Cursor gives up after 15), then allow it. Write the plan, and don't build it. Tell Shane to wait **30 seconds** after the plan appears, then switch you back to Agent mode without building it. When he has, mark `B5a end`.
2. Ask Shane to open a **new chat**, pick **Plan** in its mode picker **before** sending, and send: "Plan how to add a CHANGELOG.md. Don't ask me anything first." Then, when the plan's done, to wait **30 seconds**, then build it, and come back here when it's finished. Mark `B5b start` before he starts and `B5b end` when he's back. (If you can't mark while he's away, mark when he's back, and note the times he gives you.)
3. Ask Shane to open a new chat, pick **Ask** in its mode picker, and send: "What does slow.sh do?", then come back. Mark it the same way, as B5c.

### B6. Messages sent mid-turn (8 minutes)

**Checks:** a message sent while you're working is queued (Shane's setting), one sent to interrupt stops the turn, and the stop button stops it. A foreground subagent of a stopped turn is canceled.

For each, tell Shane what he'll do first, and end your turn. When he says go, do the work.

1. Shane will send "Also, how many files are in docs?" with plain Enter, **10 seconds** after you start. You: mark `B6a start`, start two subagents in parallel, each running `./slow.sh 30`, then report, and then answer his question. Mark `B6a end`.
2. The same, but Shane sends his follow-up so that it **interrupts** you (from the queue, or however the app offers it: he'll tell you what he did). Mark `B6b start` first. Once you're back, mark `B6b end`.
3. The same work, and Shane presses **the stop button** 10 seconds in. Mark `B6c start` first. When he tells you to carry on, mark `B6c end`.

### B7. Approvals that aren't commands (6 minutes)

**Checks:** what fires, if anything, while Cursor asks about something other than a command. Agent Graph only watches commands.

1. Tell Shane: "I'm about to write a file outside this workspace. When Cursor asks, wait 20 seconds, then refuse." Mark `B7a start`, try to write the word hello to `~/cursor-stress-outside.txt`, and mark `B7a end`.
2. Tell Shane: "I'm about to search the web. When Cursor asks, wait 20 seconds, then approve." Mark `B7b start`, search the web for the current version of Rust, report it, and mark `B7b end`.

### B8. Load from the app (6 minutes)

**Checks:** as A11, alongside your own chat, and with several of the app's own chats at once.

1. Run A11's six background print runs again, as `B8a` (mark `B8a start`, write to `drive-logs/B8-$i.log`, and wait the same way). While they run, start two subagents of your own, in parallel, to count the files in src and in docs. Mark `B8a end` when all six are done.
2. Ask Shane to open three new chats, and in each, quickly one after another, send "Run ./slow.sh 20, then reply with just: chat N done" (N = 1, 2, 3), then come back. Mark `B8b start` before and `B8b end` after.
3. Ask Shane whether Cursor offers running one prompt with several agents at once (in parallel, best of N, in worktrees). If it does, ask him to use it with "Reply with the output of pwd", and note what it was called and how many ran (B8c). If not, note that.

### B9. Ending chats (4 minutes)

**Checks:** what a chat sends as it ends (`sessionEnd` and its `reason`), when its tab is closed mid-turn, and when Cursor quits.

1. Ask Shane to open a new chat, send "Run ./slow.sh 60 and report.", and close the chat's tab 10 seconds later, then come back. Mark B9a around it.
2. **Last of all**, because it ends this chat too: tell Shane the run is over, apart from this:
   - mark `B9b start`;
   - Shane sets Run Mode back to **Auto-Review (with Sandbox)**, and empties the Command Allowlist;
   - Shane quits Cursor (⌘Q) while this chat is idle, opens it again, and runs `scripts/cursor-stress/setup.sh finish` in Terminal, from the agent-graph repository. That puts the hooks back without raw capture, and marks the finish.

   Write the notes' last lines before you hand over: a summary of anything that surprised you, with step ids.

## What the watching session will read

- `~/cursor-stress/steps.log` (every mark, timestamped) and `notes.md`;
- `~/.agent-graph/events/` (`cursor-*`, and the `claude-code-*` and `codex-*` files the steps make);
- `~/.agent-graph/raw/cursor-*.jsonl` (what Cursor sent, from setup on);
- `~/cursor-stress/drive-logs/` (the CLI's screen, step by step);
- `~/.agent-graph/emit.log` (errors from hooks).

Afterwards, Shane can delete `~/cursor-stress`, `~/cursor-stress-outside.txt` (if B7 made it after all), and the raw payloads in `~/.agent-graph/raw/`.
