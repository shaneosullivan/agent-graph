import {createHash} from "node:crypto";

import {Timestamp} from "firebase-admin/firestore";

import {firestore} from "./firebase";

/**
 * What a coding agent's cloud reports of itself (`agent-graph diagnostics`,
 * which `watch-remote` runs there as it starts and stops: the CLI's
 * src/diagnostics.rs), kept for its account:
 *
 *   cloud-diagnostics/{sha256(uid, key)}
 *     { uid, key, cloud, issues, report, at, active, muted }
 *
 * `key` is the breakage: the cloud ("Cursor cloud") and the problems that
 * stop it (`issues`, the ids of docs/troubleshooting.json's), so the same
 * breakage reported again is the same document, made `active` again with
 * the latest report. A report with no problems clears that cloud's
 * (`active` false: it works now). Dismissing one (/watch, or the local
 * viewer) clears it until it's reported again; muting it ("never show
 * this again") hides that breakage, in that cloud, until it's unmuted on
 * the account page.
 */

/** The clouds `agent-graph` names (its `account::cloud`). */
export const CLOUDS = ["Claude Code cloud", "Codex cloud", "Cursor cloud"];

/** The largest report kept: the CLI's are a few KB. */
export const MOST_REPORT_BYTES = 64 * 1024;

/** The most breakages kept for an account: the oldest go first. */
const MOST_PER_ACCOUNT = 50;

const reports = () => firestore().collection("cloud-diagnostics");

const docId = (uid: string, key: string) =>
  createHash("sha256").update(`${uid}\n${key}`).digest("hex");

/** One finding, as the CLI sends it. */
export type Finding = {
  level: "ok" | "info" | "warn" | "error";
  what: string;
  issue?: string;
  detail?: string;
};

/** A report, as the CLI sends it (src/diagnostics.rs's `Report`). */
export type Report = {
  version: string;
  cloud?: string;
  platform: string;
  site: string;
  at: string;
  findings: Array<Finding>;
  logs?: Record<string, string>;
};

/** A breakage, as the viewer and the account page show it. */
export type Breakage = {
  key: string;
  cloud: string;
  issues: Array<string>;
  /** When it was last reported (ms). */
  at: number;
  report: Report;
};

const ISSUE = /^[a-z][a-z-]{0,63}$/;

/**
 * `body` as a report from a cloud, if it's one: what's kept of it is only
 * what the CLI sends, each string cut to a sensible length.
 */
export function parseReport(body: unknown): Report | null {
  if (!body || typeof body !== "object") {
    return null;
  }
  const b = body as Record<string, unknown>;
  const str = (v: unknown, max: number) =>
    typeof v === "string" ? v.slice(0, max) : "";
  if (typeof b.cloud !== "string" || !CLOUDS.includes(b.cloud)) {
    return null;
  }
  if (!Array.isArray(b.findings) || b.findings.length > 100) {
    return null;
  }
  const findings: Array<Finding> = [];
  for (const f of b.findings as Array<Record<string, unknown>>) {
    if (!f || typeof f !== "object") {
      return null;
    }
    const level = f.level;
    if (
      level !== "ok" &&
      level !== "info" &&
      level !== "warn" &&
      level !== "error"
    ) {
      return null;
    }
    const issue =
      typeof f.issue === "string" && ISSUE.test(f.issue) ? f.issue : undefined;
    findings.push({
      level,
      what: str(f.what, 500),
      ...(issue ? {issue} : {}),
      ...(typeof f.detail === "string" ? {detail: str(f.detail, 2000)} : {}),
    });
  }
  const logs: Record<string, string> = {};
  if (b.logs && typeof b.logs === "object") {
    for (const [name, text] of Object.entries(b.logs).slice(0, 5)) {
      if (/^[a-z.-]{1,40}$/.test(name) && typeof text === "string") {
        logs[name] = text.slice(-8000);
      }
    }
  }
  return {
    version: str(b.version, 40),
    cloud: b.cloud,
    platform: str(b.platform, 80),
    site: str(b.site, 200),
    at: str(b.at, 40),
    findings,
    ...(Object.keys(logs).length ? {logs} : {}),
  };
}

/** The problems that stop `report`'s cloud, each once, sorted. */
export function issuesOf(report: Report): Array<string> {
  const ids = report.findings
    .filter(f => f.level === "error" && f.issue)
    .map(f => f.issue as string);
  return [...new Set(ids)].sort();
}

/** The key a breakage of `issues` in `cloud` is kept by. */
export function keyOf(cloud: string, issues: Array<string>): string {
  return `${cloud}:${issues.join(",")}`;
}

/**
 * Keeps a report from one of account `uid`'s clouds: its breakage made
 * active again (muted or not), or for a report with no problems, the
 * cloud's breakages cleared. Returns the breakage's key, or null if there
 * was none.
 */
export async function recordReport(
  uid: string,
  report: Report,
): Promise<string | null> {
  const cloud = report.cloud as string;
  const issues = issuesOf(report);
  if (issues.length === 0) {
    const active = await reports()
      .where("uid", "==", uid)
      .where("cloud", "==", cloud)
      .where("active", "==", true)
      .get();
    await Promise.all(active.docs.map(d => d.ref.update({active: false})));
    return null;
  }
  const key = keyOf(cloud, issues);
  const ref = reports().doc(docId(uid, key));
  await firestore().runTransaction(async tx => {
    const snap = await tx.get(ref);
    tx.set(ref, {
      uid,
      key,
      cloud,
      issues,
      report: JSON.stringify(report),
      at: Timestamp.now(),
      active: true,
      muted: snap.exists ? snap.get("muted") === true : false,
    });
  });
  await keepTheLatest(uid);
  return key;
}

/** Deletes account `uid`'s oldest breakages past the most it keeps. */
async function keepTheLatest(uid: string): Promise<void> {
  const all = await reports().where("uid", "==", uid).get();
  if (all.size <= MOST_PER_ACCOUNT) {
    return;
  }
  const oldest = all.docs
    .sort((a, b) => a.get("at").toMillis() - b.get("at").toMillis())
    .slice(0, all.size - MOST_PER_ACCOUNT);
  await Promise.all(oldest.map(d => d.ref.delete()));
}

function breakage(doc: FirebaseFirestore.QueryDocumentSnapshot): Breakage {
  let report: Report;
  try {
    report = JSON.parse(doc.get("report")) as Report;
  } catch {
    report = {
      version: "",
      platform: "",
      site: "",
      at: "",
      findings: [],
    };
  }
  return {
    key: doc.get("key"),
    cloud: doc.get("cloud"),
    issues: doc.get("issues") ?? [],
    at: (doc.get("at") as Timestamp).toMillis(),
    report,
  };
}

/**
 * Account `uid`'s breakages to show: those reported and not since cleared,
 * dismissed or muted (or with `muted`, every muted one, for the account
 * page), the latest first.
 */
export async function breakagesOf(
  uid: string,
  muted = false,
): Promise<Array<Breakage>> {
  const snap = await reports().where("uid", "==", uid).get();
  return snap.docs
    .filter(d =>
      muted
        ? d.get("muted") === true
        : d.get("active") === true && d.get("muted") !== true,
    )
    .map(breakage)
    .sort((a, b) => b.at - a.at);
}

export type Action = "dismiss" | "mute" | "unmute";

/**
 * Dismisses (until it's reported again), mutes (until unmuted) or unmutes
 * account `uid`'s breakage `key`. Whether it's one of theirs.
 */
export async function changeBreakage(
  uid: string,
  key: string,
  action: Action,
): Promise<boolean> {
  const ref = reports().doc(docId(uid, key));
  const snap = await ref.get();
  if (!snap.exists || snap.get("uid") !== uid) {
    return false;
  }
  await ref.update(
    action === "dismiss"
      ? {active: false}
      : action === "mute"
        ? {muted: true}
        : {muted: false},
  );
  return true;
}

/** Deletes everything kept of account `uid`'s clouds' reports. */
export async function deleteBreakagesOf(uid: string): Promise<void> {
  const mine = await reports().where("uid", "==", uid).get();
  await Promise.all(mine.docs.map(d => d.ref.delete()));
}
