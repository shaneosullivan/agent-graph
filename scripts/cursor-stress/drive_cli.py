#!/usr/bin/env python3
"""Drives Cursor's CLI (`agent`) through a pseudo-terminal, as a person at a
terminal would: starts it with a prompt, answers what it asks after a set
delay, and quits once it has been quiet for a while. For the Cursor stress
test (docs/cursor-stress-test.md).

    drive_cli.py --step A3 --prompt "Run date" \\
        --rule 'Run this command|Allow=>20=>{enter}' --quiet 15

  --step NAME       marks "NAME start" and "NAME end" in ~/cursor-stress/steps.log,
                    and logs the screen to ~/cursor-stress/drive-logs/NAME.log
  --prompt TEXT     the first prompt (passed to `agent` as its argument)
  --arg ARG         more arguments for `agent` (repeatable: --arg=--force)
  --rule R=>S=>K    when the screen matches regex R, wait S seconds, then send
                    keys K. Each rule fires once, in turn: list them in the
                    order they should happen. Keys are text, with {enter},
                    {esc}, {tab}, {up}, {down}, {left}, {right}, {space},
                    {ctrl-c}, {ctrl-d}
  --say S=>TEXT     S seconds after start, type TEXT and press enter
                    (a follow-up message; repeatable)
  --quiet SECS      quit once the screen has been still this long after the
                    last rule has fired (default 20)
  --quit KEYS       keys that quit (default {ctrl-c}{ctrl-c})
  --timeout SECS    give up after this long (default 300)
  --screen          also print what the screen shows as it goes, for finding
                    out what Cursor's prompts say and which keys answer them

Only the standard library. The log is the screen's text with its escape
codes taken out, so a redrawn screen shows up more than once.
"""

import argparse
import os
import pty
import re
import select
import signal
import subprocess
import sys
import time

WS = os.environ.get("DRIVE_WS") or os.path.expanduser("~/cursor-stress")
AGENT = os.environ.get("DRIVE_AGENT") or os.path.expanduser("~/.local/bin/agent")
KEYS = {
    "enter": "\r",
    "esc": "\x1b",
    "tab": "\t",
    "up": "\x1b[A",
    "down": "\x1b[B",
    "right": "\x1b[C",
    "left": "\x1b[D",
    "space": " ",
    "ctrl-c": "\x03",
    "ctrl-d": "\x04",
}
ANSI = re.compile(r"\x1b\[[0-?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(\x07|\x1b\\)|\x1b[@-Z\\-_]")


def keys(spec):
    return re.sub(r"\{([a-z-]+)\}", lambda m: KEYS.get(m.group(1), m.group(0)), spec)


def mark(text):
    subprocess.run([os.path.join(WS, "mark"), text], stdout=subprocess.DEVNULL)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--step", required=True)
    ap.add_argument("--prompt")
    ap.add_argument("--arg", action="append", default=[])
    ap.add_argument("--rule", action="append", default=[])
    ap.add_argument("--say", action="append", default=[])
    ap.add_argument("--quiet", type=float, default=20)
    ap.add_argument("--quit", default="{ctrl-c}{ctrl-c}")
    ap.add_argument("--timeout", type=float, default=300)
    ap.add_argument("--screen", action="store_true")
    ap.add_argument("--cwd", default=WS)
    a = ap.parse_args()

    rules = []
    for r in a.rule:
        pattern, delay, send = r.split("=>", 2)
        rules.append((re.compile(pattern, re.I), float(delay), keys(send)))
    says = sorted((float(s.split("=>", 1)[0]), s.split("=>", 1)[1]) for s in a.say)

    os.makedirs(os.path.join(WS, "drive-logs"), exist_ok=True)
    log = open(os.path.join(WS, "drive-logs", f"{a.step}.log"), "a")
    argv = [AGENT, *a.arg] + ([a.prompt] if a.prompt else [])
    env = dict(os.environ, TERM="xterm-256color", COLUMNS="120", LINES="40")
    # A chat started from a Cursor chat's shell (the runbook's) keeps its
    # CURSOR_CONVERSATION_ID, so it's linked under that chat. Nothing else
    # that would link it elsewhere.
    for var in ("AGENT_GRAPH_PARENT", "AGENT_GRAPH_PARENT_CODEX", "AGENT_GRAPH_PARENT_CURSOR",
                "CODEX_THREAD_ID", "CODEX_SESSION_ID", "TRACEPARENT"):
        env.pop(var, None)

    mark(f"{a.step} start: agent {' '.join(argv[1:])[:200]}")
    pid, fd = pty.fork()
    if pid == 0:
        os.chdir(a.cwd)
        os.execve(AGENT, argv, env)

    start = last_change = time.time()
    screen = ""
    pending = None  # (when, keys, label)
    fired = 0
    quitting = None
    while True:
        now = time.time()
        if now - start > a.timeout:
            mark(f"{a.step} timeout after {a.timeout:.0f}s")
            break
        r, _, _ = select.select([fd], [], [], 0.2)
        if r:
            try:
                data = os.read(fd, 65536)
            except OSError:
                break
            if not data:
                break
            text = ANSI.sub("", data.decode("utf-8", "replace")).replace("\r", "")
            if text.strip():
                log.write(text)
                log.flush()
                if a.screen:
                    sys.stdout.write(text)
                    sys.stdout.flush()
                screen = (screen + text)[-6000:]
                last_change = now
        # The next rule, once its text is on screen.
        if pending is None and fired < len(rules):
            pattern, delay, send = rules[fired]
            if pattern.search(screen):
                pending = (now + delay, send, pattern.pattern)
                mark(f"{a.step} saw /{pattern.pattern}/: answering in {delay:.0f}s")
        if pending and now >= pending[0]:
            os.write(fd, pending[1].encode())
            mark(f"{a.step} answered /{pending[2]}/ with {pending[1]!r}")
            screen = ""  # match the next rule on what comes after
            pending = None
            fired += 1
        if says and now - start >= says[0][0]:
            _, text = says.pop(0)
            os.write(fd, (text + "\r").encode())
            mark(f"{a.step} typed: {text[:120]}")
        done = fired == len(rules) and pending is None and not says
        if done and quitting is None and now - last_change > a.quiet:
            os.write(fd, keys(a.quit).encode())
            mark(f"{a.step} quitting after {a.quiet:.0f}s quiet")
            quitting = now
        if quitting and now - quitting > 10:
            os.kill(pid, signal.SIGTERM)
            break
    try:
        os.waitpid(pid, 0)
    except ChildProcessError:
        pass
    if fired < len(rules):
        mark(f"{a.step} NOTE: only {fired} of {len(rules)} rules fired")
    mark(f"{a.step} end")


if __name__ == "__main__":
    main()
