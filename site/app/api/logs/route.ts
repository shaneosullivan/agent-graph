import {
  bodyText,
  clientAddress,
  MAX_CHUNK_BYTES,
  MAX_PASSWORD_BYTES,
  SOURCES,
  type Source,
  siteUrl,
} from "@/lib/config";
import {accountOfRequest, setWatchLog, standingOf} from "@/lib/accounts";
import {mustSubscribe, standingReply} from "@/lib/billing";
import {
  hashPassword,
  newId,
  PasswordTooLong,
  passwordFromHeader,
  writeToken,
} from "@/lib/crypto";
import {IdTaken, createLog, takeScryptRun} from "@/lib/store";
import {tooMany} from "@/lib/unlock";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

/**
 * Creates a log.
 *
 * Body: the first chunk of JSON Lines, raw (at most MAX_CHUNK_BYTES).
 * Headers:
 *   X-Agent-Graph-Source: watch | paste | upload
 *   X-Agent-Graph-Password: base64url(UTF-8 password), optional (at most
 *   MAX_PASSWORD_BYTES)
 *   Authorization: Bearer <CLI token>: an account's live share
 *   (`agent-graph watch-remote`, logged in: lib/accounts.ts), which only
 *   that account can view, at /watch. A live share (source `watch`) must
 *   have one; it can't have a password as well.
 * Reply (201): { id, url, writeToken }, and, for an account's share, where
 * it stands: { accountStatus, freeUntil } (lib/billing.ts). 402 if it has
 * to subscribe first; appends to it are refused with 402 once it has to
 * (and it's started again: /api/logs/{id}/watch). Send further chunks to
 * /api/logs/{id}/append with the write token. 429 (with Retry-After) if the
 * address has run scrypt too often lately (lib/config.ts).
 */
export async function POST(req: Request): Promise<Response> {
  if (Number(req.headers.get("content-length") ?? 0) > MAX_CHUNK_BYTES) {
    return tooLarge();
  }
  const text = await bodyText(req, MAX_CHUNK_BYTES);
  if (text === null) {
    return tooLarge();
  }

  const header = req.headers.get("x-agent-graph-source") as Source | null;
  const source: Source = header && SOURCES.includes(header) ? header : "paste";

  const account = req.headers.has("authorization")
    ? await accountOfRequest(req)
    : null;
  if (req.headers.has("authorization") && !account) {
    return new Response(
      "That login has ended. Log in again: agent-graph watch-remote asks you to.",
      {
        status: 401,
      },
    );
  }
  if (source === "watch" && !account) {
    return new Response(
      "Sharing live needs you to be logged in. Update agent-graph, and run it again.",
      {
        status: 401,
      },
    );
  }
  if (account && req.headers.has("x-agent-graph-password")) {
    return new Response(
      "A share of your own is private to your account: it can't have a password.",
      {
        status: 400,
      },
    );
  }

  let password: string | null;
  try {
    password = passwordFromHeader(req);
  } catch (err) {
    if (err instanceof PasswordTooLong) {
      return new Response(
        `A password may be at most ${MAX_PASSWORD_BYTES} bytes.`,
        {status: 400},
      );
    }
    return new Response("Malformed X-Agent-Graph-Password header.", {
      status: 400,
    });
  }
  // Hashing it is a scrypt run, which is limited per address.
  if (password) {
    const run = await takeScryptRun(clientAddress(req));
    if ("wait" in run) {
      return tooMany("Too many passwords from this address", run.wait);
    }
  }
  const pw = password ? await hashPassword(password) : undefined;

  const standing = account ? await standingOf(account.uid) : null;
  if (standing && !standing.canShare) {
    return mustSubscribe(siteUrl(req));
  }

  for (let attempt = 0; attempt < 3; attempt++) {
    const id = newId();
    try {
      await createLog(
        id,
        {source, pw, ...(account ? {owner: account.uid} : {})},
        text,
        standing?.until ?? null,
      );
    } catch (err) {
      if (err instanceof IdTaken) {
        continue;
      }
      throw err;
    }
    if (account) {
      await setWatchLog(account.uid, id);
    }
    const url = account ? `${siteUrl(req)}/watch` : `${siteUrl(req)}/l/${id}`;
    return Response.json(
      {
        id,
        url,
        writeToken: writeToken(id),
        ...(standing ? standingReply(standing) : {}),
      },
      {status: 201, headers: {"Cache-Control": "no-store"}},
    );
  }
  return new Response("Couldn't allocate an id; try again.", {status: 503});
}

function tooLarge(): Response {
  return new Response(
    `Each request may carry at most ${MAX_CHUNK_BYTES} bytes.`,
    {status: 413},
  );
}
