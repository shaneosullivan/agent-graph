"use client";

import { useMemo, useState } from "react";

import { eventsShared, forSite } from "@/lib/trim";
import { type Summary, share, summarize } from "@/lib/upload";

type Mode = "paste" | "upload";

export function Uploader() {
  const [mode, setMode] = useState<Mode>("paste");
  const [pasted, setPasted] = useState("");
  const [files, setFiles] = useState<{ name: string; size: number; text: string }[]>([]);
  const [password, setPassword] = useState("");
  const [over, setOver] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [progress, setProgress] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  const text = mode === "paste" ? pasted : files.map((f) => f.text.trimEnd()).join("\n");
  const summary = useMemo<Summary | null>(() => (text.trim() ? summarize(text) : null), [text]);
  const ready = !!summary && summary.events > 0 && !busy;

  async function addFiles(list: FileList | null) {
    if (!list?.length) return;
    const read = await Promise.all(
      Array.from(list).map(async (f) => ({ name: f.name, size: f.size, text: await f.text() })),
    );
    setFiles((prev) => [...prev, ...read]);
    setError(null);
  }

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    if (!ready || !summary) return;
    setError(null);
    try {
      // Only its last two keyframes' worth: the site keeps no more.
      setBusy("Preparing…");
      setProgress(0);
      const shared = await forSite(text, summary.events, setProgress);
      setBusy("Creating your link…");
      setProgress(null);
      const id = await share(shared.text, {
        source: mode,
        password: password || undefined,
        onProgress: (sent, total) => {
          if (total <= 1) return;
          setBusy(`Uploading… ${sent} of ${total}`);
          setProgress(sent / total);
        },
      });
      setBusy("Opening…");
      // A full page load: the viewer is a separate app with its own scripts.
      window.location.assign(`/l/${id}`);
    } catch (err) {
      setError((err as Error).message);
      setBusy(null);
      setProgress(null);
    }
  }

  return (
    <form className="card" onSubmit={submit}>
      <div className="tabs" role="tablist">
        <button
          type="button"
          role="tab"
          className="tab"
          aria-selected={mode === "paste"}
          onClick={() => setMode("paste")}
        >
          Paste
        </button>
        <button
          type="button"
          role="tab"
          className="tab"
          aria-selected={mode === "upload"}
          onClick={() => setMode("upload")}
        >
          Upload files
        </button>
      </div>
      <div className="card-body">
        {mode === "paste" ? (
          <textarea
            className="paste"
            value={pasted}
            onChange={(e) => setPasted(e.target.value)}
            placeholder={
              '{"v":1,"id":"01K…","ts":"2026-09-25T10:14:03.221Z","type":"session.started","node":"claude-code:5f2c…",…}\n…'
            }
            spellCheck={false}
            aria-label="Agent Graph log (JSON Lines)"
          />
        ) : (
          <>
            <label
              className={`drop${over ? " over" : ""}`}
              onDragOver={(e) => {
                e.preventDefault();
                setOver(true);
              }}
              onDragLeave={() => setOver(false)}
              onDrop={(e) => {
                e.preventDefault();
                setOver(false);
                addFiles(e.dataTransfer.files);
              }}
            >
              <input
                type="file"
                multiple
                accept=".jsonl,.json,.ndjson,.txt,application/x-ndjson,text/plain"
                onChange={(e) => addFiles(e.target.files)}
              />
              <strong>Drop log files here, or click to choose</strong>
              <span>
                One or more <code>.jsonl</code> files from <code>~/.agent-graph/events/</code>
              </span>
            </label>
            {files.length > 0 && (
              <ul className="files">
                {files.map((f, i) => (
                  <li key={`${f.name}-${i}`}>
                    <span>{f.name}</span>
                    <span>{(f.size / 1024).toFixed(1)} KB</span>
                  </li>
                ))}
              </ul>
            )}
          </>
        )}

        <p className={`summary${summary && summary.events === 0 ? " bad" : ""}`} aria-live="polite">
          {summary &&
            (summary.events === 0 ? (
              "That doesn't look like an Agent Graph log: no events found."
            ) : (
              <>
                <span className="ok">
                  {summary.events} event{summary.events === 1 ? "" : "s"} in {summary.sessions} session
                  {summary.sessions === 1 ? "" : "s"}
                </span>
                {summary.skipped > 0 &&
                  ` · ${summary.skipped} line${summary.skipped === 1 ? "" : "s"} skipped`}
                {eventsShared(summary.events) < summary.events &&
                  ` · the last ${eventsShared(summary.events)} are shared: the site keeps a log's last two keyframes' worth`}
              </>
            ))}
        </p>

        <div className="row">
          <label className="field">
            Password <span className="hint">Optional. Viewers will need it.</span>
            <input
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              autoComplete="new-password"
              maxLength={200}
            />
          </label>
          <button className="button" type="submit" disabled={!ready}>
            {busy ?? "Create shareable link"}
          </button>
        </div>
        {progress !== null && <progress className="progress" max={1} value={progress} aria-label={busy ?? "Working"} />}
        {error && <p className="error">{error}</p>}
      </div>
    </form>
  );
}
