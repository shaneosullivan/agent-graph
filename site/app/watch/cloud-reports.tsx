"use client";

import {useEffect, useRef, useState} from "react";

/**
 * What the account's coding agents' clouds report is wrong with them, on
 * /watch before anything's been shared (a broken cloud often hasn't): the
 * same alerts, and the same dialog, as the viewer shows atop its sessions
 * list once there's a share (src/view/assets/app.js).
 */

type Finding = {level: string; what: string; issue?: string; detail?: string};
type Fix = {id: string; title: string; fix: string; link: string};
type Breakage = {
  key: string;
  cloud: string;
  issues: Array<string>;
  at: number;
  report: {
    version?: string;
    platform?: string;
    findings?: Array<Finding>;
    logs?: Record<string, string>;
  };
  fixes?: Array<Fix>;
};

/** `text` with its `backticks` as code. */
function WithCode({text}: {text: string}) {
  return (
    <>
      {text
        .split("`")
        .map((part, i) =>
          i % 2 ? <code key={i}>{part}</code> : <span key={i}>{part}</span>,
        )}
    </>
  );
}

export function CloudReports() {
  const [reports, setReports] = useState<Array<Breakage>>([]);
  const [open, setOpen] = useState<Breakage | null>(null);
  const [error, setError] = useState<string | null>(null);
  const dialog = useRef<HTMLDialogElement>(null);

  useEffect(() => {
    let live = true;
    const load = () =>
      fetch("/api/cloud-diagnostics", {cache: "no-store"})
        .then(r => (r.ok ? r.json() : {reports: []}))
        .then(b => live && setReports(b.reports ?? []))
        .catch(() => {});
    load();
    const every = setInterval(load, 5 * 60_000);
    return () => {
      live = false;
      clearInterval(every);
    };
  }, []);

  useEffect(() => {
    if (open && dialog.current && !dialog.current.open) {
      dialog.current.showModal();
    }
  }, [open]);

  async function act(key: string, action: "dismiss" | "mute") {
    setError(null);
    const res = await fetch("/api/cloud-diagnostics", {
      method: "PATCH",
      headers: {"Content-Type": "application/json"},
      body: JSON.stringify({key, action}),
    });
    if (!res.ok) {
      setError(`That didn't work: ${await res.text()}`);
      return;
    }
    setReports(rs => rs.filter(r => r.key !== key));
    dialog.current?.close();
  }

  if (!reports.length) {
    return null;
  }
  const findings = open?.report.findings ?? [];
  const fixes = new Map((open?.fixes ?? []).map(f => [f.id, f]));
  return (
    <div className="cloud-reports" role="alert">
      {reports.map(r => (
        <button
          key={r.key}
          type="button"
          className="cloud-report-alert"
          onClick={() => setOpen(r)}>
          <strong>{r.cloud} isn&rsquo;t working</strong>
          <span>
            {r.issues.length} problem{r.issues.length === 1 ? "" : "s"},
            reported {new Date(r.at).toLocaleString()}. What&rsquo;s wrong?
          </span>
        </button>
      ))}
      <dialog
        ref={dialog}
        className="cloud-report-modal"
        onClose={() => setOpen(null)}
        onClick={e => {
          if (e.target === dialog.current) {
            dialog.current?.close();
          }
        }}>
        {open ? (
          <div className="cloud-report-box">
            <button
              type="button"
              className="cloud-report-close"
              aria-label="Close"
              onClick={() => dialog.current?.close()}>
              ×
            </button>
            <h2>{open.cloud} isn&rsquo;t working</h2>
            <p className="muted">
              Reported {new Date(open.at).toLocaleString()} by agent-graph{" "}
              {open.report.version}
              {open.report.platform ? ` (${open.report.platform})` : ""}. Fix
              what&rsquo;s below, then start a new session there: a cloud that
              works again clears this by itself.
            </p>
            <h3>What&rsquo;s wrong</h3>
            {findings
              .filter(f => f.level === "error" || f.level === "warn")
              .map((f, i) => {
                const fix = f.issue ? fixes.get(f.issue) : undefined;
                return (
                  <div
                    key={i}
                    className={`cloud-report-finding level-${f.level}`}>
                    <p className="cloud-report-what">
                      {f.level === "error" ? "✗" : "!"} {f.what}
                    </p>
                    {f.detail ? <pre>{f.detail}</pre> : null}
                    {fix ? (
                      <>
                        <p>
                          <strong>Fix:</strong> <WithCode text={fix.fix} />
                        </p>
                        <a href={fix.link} target="_blank" rel="noopener">
                          More: {fix.title}
                        </a>
                      </>
                    ) : null}
                  </div>
                );
              })}
            {findings.some(f => f.level === "ok" || f.level === "info") ? (
              <>
                <h3>What&rsquo;s in place</h3>
                <ul className="cloud-report-fine">
                  {findings
                    .filter(f => f.level === "ok" || f.level === "info")
                    .map((f, i) => (
                      <li key={i}>
                        {f.level === "ok" ? "✓" : "•"} {f.what}
                      </li>
                    ))}
                </ul>
              </>
            ) : null}
            {Object.entries(open.report.logs ?? {}).map(([name, text]) => (
              <div key={name}>
                <h3>The end of {name}</h3>
                <pre>{text}</pre>
              </div>
            ))}
            <div className="cloud-report-actions">
              <button
                type="button"
                className="button"
                title="Hide it until this cloud reports it again."
                onClick={() => act(open.key, "dismiss")}>
                Dismiss
              </button>
              <button
                type="button"
                className="button secondary"
                title={`Never show this problem in ${open.cloud} again. Your account page can show it again.`}
                onClick={() => act(open.key, "mute")}>
                Never show this again
              </button>
            </div>
            {error ? <p className="error">{error}</p> : null}
          </div>
        ) : null}
      </dialog>
    </div>
  );
}
