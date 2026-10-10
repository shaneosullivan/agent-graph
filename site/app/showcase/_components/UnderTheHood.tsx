"use client";

import {useState} from "react";

import {useCallLog} from "@/app/showcase/_lib/client";
import {useSource} from "@/app/showcase/_lib/source";

/**
 * Every call this page made to the API: what it asked for, what came back,
 * and how long it took. Pick one to get it as a curl command.
 */
export function UnderTheHood({apiUrl}: {apiUrl: string}) {
  const {calls} = useCallLog();
  const {source} = useSource();
  const [picked, setPicked] = useState<number | null>(null);
  const call = calls.find(c => c.id === picked) ?? calls[0];
  const total = calls.reduce((n, c) => n + c.ms, 0);

  return (
    <details className="hood">
      <summary>
        <span style={{fontSize: 16}}>⚙</span>
        <b>Under the hood</b>
        <span>
          {calls.length
            ? `this page made ${calls.length} API call${calls.length === 1 ? "" : "s"}, ${total} ms in all`
            : "no API calls yet"}
          {calls.some(c => c.notModified)
            ? ` · ${calls.filter(c => c.notModified).length} answered 304 Not Modified`
            : ""}
        </span>
      </summary>
      <div className="hood-body">
        <p className="muted" style={{fontSize: 13, marginTop: 0}}>
          Each row is one request to <code>{apiUrl}</code>, made by this page,
          in your browser,{" "}
          {source === "demo"
            ? "with the demo\u2019s key"
            : "with a key made for your account when you switched, which lasts an hour"}
          . Click one to copy it as a <code>curl</code> command and run it
          yourself, with a key of your own.
        </p>
        {calls.map(c => (
          <div
            key={c.id}
            className="call"
            onClick={() => setPicked(c.id)}
            style={{
              cursor: "pointer",
              background: c === call ? "rgba(124,140,255,0.06)" : undefined,
            }}>
            <span
              className="status"
              style={{
                color:
                  c.status === 304
                    ? "var(--accent-2)"
                    : c.status < 400
                      ? "var(--good)"
                      : "var(--bad)",
              }}>
              {c.status}
            </span>
            <span className="path">GET {c.path}</span>
            <span className="faint mono">
              {c.requestId?.slice(0, 14) ?? ""}
            </span>
            <span className="muted mono">{c.ms} ms</span>
          </div>
        ))}
        {call ? (
          <pre className="curl">{`curl -G "${apiUrl}${call.path.replace(/"/g, '\\"')}" \\
  -H "Authorization: Bearer $AGENT_GRAPH_KEY"`}</pre>
        ) : null}
      </div>
    </details>
  );
}
