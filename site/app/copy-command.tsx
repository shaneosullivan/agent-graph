"use client";

import {useEffect, useRef, useState} from "react";

/** How long the tick shows after copying, before the copy icon's back. */
const COPIED_MS = 2000;

/**
 * A command to type in a terminal, with a button that copies it: at the
 * top right of a command of several lines, and centred on the right of a
 * one-line one. The button shows a tick for a moment once it's copied, and
 * `onCopy` is called (to count it: app/install.tsx).
 */
export function CopyCommand({
  command,
  onCopy,
}: {
  command: string;
  onCopy?: () => void;
}) {
  const [copied, setCopied] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => () => clearTimeout(timer.current), []);

  async function copy() {
    try {
      await navigator.clipboard.writeText(command);
    } catch {
      // Not allowed (an insecure page, say): nothing's copied, so no tick.
      return;
    }
    setCopied(true);
    onCopy?.();
    clearTimeout(timer.current);
    timer.current = setTimeout(() => setCopied(false), COPIED_MS);
  }

  const lines = command.includes("\n") ? "multi" : "single";
  return (
    <span className={`command-box ${lines}`}>
      <code className="command">{command}</code>
      <button
        type="button"
        className="copy-command"
        onClick={copy}
        aria-label={copied ? "Copied" : "Copy to clipboard"}
        title="Copy to clipboard">
        <img
          src={copied ? "/images/check.png" : "/images/copy.png"}
          alt=""
          width={16}
          height={16}
        />
      </button>
    </span>
  );
}
