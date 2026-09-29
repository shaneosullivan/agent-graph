import {createHmac, randomBytes, scrypt, timingSafeEqual} from "node:crypto";

import {MAX_PASSWORD_BYTES} from "./config";

/**
 * Ids, tokens and password hashes.
 *
 * Write tokens are an HMAC of the log id, so an append can be authorised by
 * recomputing it: no database read, which keeps the hot path cheap. Viewer
 * cookies are an HMAC of the id and the stored password hash.
 */

let warned = false;

function secret(): string {
  const value = process.env.AGENT_GRAPH_SECRET;
  if (value) {
    return value;
  }
  if (process.env.NODE_ENV === "production") {
    throw new Error("AGENT_GRAPH_SECRET must be set in production");
  }
  if (!warned) {
    console.warn(
      "AGENT_GRAPH_SECRET isn't set; using an insecure development secret.",
    );
    warned = true;
  }
  return "agent-graph-development-secret";
}

const BASE62 = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/** A random 12-character base62 id. */
export function newId(): string {
  let id = "";
  while (id.length < 12) {
    for (const byte of randomBytes(16)) {
      // 248 = 62 * 4: rejecting larger bytes keeps every character equally likely.
      if (byte < 248 && id.length < 12) {
        id += BASE62[byte % 62];
      }
    }
  }
  return id;
}

function hmac(message: string): string {
  return createHmac("sha256", secret()).update(message).digest("base64url");
}

export function writeToken(id: string): string {
  return hmac(`write:${id}`);
}

/**
 * An address (and a log, for counting its guesses at that log), as it's
 * stored to count password guesses: keyed, so it can't be read back.
 */
export function addressKey(address: string, log?: string): string {
  return hmac(log ? `address:${address}:log:${log}` : `address:${address}`);
}

export function viewToken(id: string, passwordHash: string): string {
  return hmac(`view:${id}:${passwordHash}`);
}

/** Compares secrets without leaking how much of them matched. */
export function safeEqual(a: string | null | undefined, b: string): boolean {
  if (!a) {
    return false;
  }
  const x = Buffer.from(a);
  const y = Buffer.from(b);
  return x.length === y.length && timingSafeEqual(x, y);
}

/** Checks `Authorization: Bearer <write token>` for log `id`. */
export function canWrite(req: Request, id: string): boolean {
  const header = req.headers.get("authorization") ?? "";
  const token = header.startsWith("Bearer ") ? header.slice(7).trim() : null;
  return safeEqual(token, writeToken(id));
}

// scrypt with the parameters the Node docs recommend for interactive logins:
// about 50 ms per hash, paid only when a log is created or unlocked.
const SCRYPT = {N: 16384, r: 8, p: 1} as const;
const KEY_BYTES = 32;

function scryptAsync(password: string, salt: Buffer): Promise<Buffer> {
  return new Promise((resolve, reject) =>
    scrypt(password, salt, KEY_BYTES, SCRYPT, (err, key) =>
      err ? reject(err) : resolve(key),
    ),
  );
}

export async function hashPassword(password: string): Promise<string> {
  const salt = randomBytes(16);
  const key = await scryptAsync(password, salt);
  return `s1$${salt.toString("base64url")}$${key.toString("base64url")}`;
}

export async function verifyPassword(
  password: string,
  stored: string,
): Promise<boolean> {
  const [version, salt, key] = stored.split("$");
  if (version !== "s1" || !salt || !key) {
    return false;
  }
  const actual = await scryptAsync(password, Buffer.from(salt, "base64url"));
  return safeEqual(actual.toString("base64url"), key);
}

export class PasswordTooLong extends Error {}

/**
 * Reads the `X-Agent-Graph-Password` header: base64url-encoded UTF-8 (headers
 * must be ASCII). Returns null when absent, and throws when malformed, or
 * (`PasswordTooLong`) longer than `MAX_PASSWORD_BYTES`, which unlocking
 * wouldn't check.
 */
export function passwordFromHeader(req: Request): string | null {
  const value = req.headers.get("x-agent-graph-password");
  if (!value) {
    return null;
  }
  if (!/^[A-Za-z0-9_-]+$/.test(value)) {
    throw new Error("malformed password header");
  }
  if (value.length > Math.ceil((MAX_PASSWORD_BYTES * 4) / 3)) {
    throw new PasswordTooLong();
  }
  // (Measured as decoded: bytes that aren't UTF-8 become U+FFFD, which is what's hashed.)
  const password = Buffer.from(value, "base64url").toString("utf8");
  if (!password) {
    throw new Error("empty password");
  }
  if (Buffer.byteLength(password) > MAX_PASSWORD_BYTES) {
    throw new PasswordTooLong();
  }
  return password;
}
