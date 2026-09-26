import { createCipheriv, createDecipheriv, createHmac, hkdfSync, randomBytes } from "node:crypto";

/**
 * Encrypts log chunks before they're stored, so Firestore only ever holds
 * ciphertext. (Google already encrypts Firestore's disks; this keeps logs
 * unreadable to anyone who can read the database itself: the Firebase
 * console, exports and backups, or a leaked service-account key.)
 *
 * - AES-256-GCM: authenticated, so any change to stored data is detected.
 * - Each log has its own key, derived with HKDF from the master key
 *   AGENT_GRAPH_ENCRYPTION_KEY and the log's id.
 * - Each chunk is bound to its log and position (the GCM associated data),
 *   so chunks can't be moved between logs or reordered without detection.
 * - Logs are stored under an HMAC of their id (`storageId`), not the id: the
 *   id is the link that opens a log, so the database mustn't hold it.
 * - Each log's metadata carries a MAC (`metaTag`) bound to its id, so it
 *   can't be changed (a password removed, say) or copied from another log.
 *
 * Stored layout: [version: 1 byte][iv: 12 bytes][ciphertext][tag: 16 bytes].
 *
 * The master key is separate from AGENT_GRAPH_SECRET (which signs write keys
 * and cookies): rotating one doesn't touch the other. Changing the master key
 * makes existing logs unreadable, so treat it like the data itself. Nor can
 * logs be re-encrypted under a new key: that needs their ids, which aren't
 * stored (so the key and the database alone don't open anything either).
 */

const VERSION = 1;
const IV_BYTES = 12;
const TAG_BYTES = 16;
const KEY_BYTES = 32;

let master: Buffer | undefined;

function masterKey(): Buffer {
  if (master) return master;
  const value = process.env.AGENT_GRAPH_ENCRYPTION_KEY;
  if (value) {
    const key = Buffer.from(value, "base64url");
    if (key.length !== KEY_BYTES) {
      throw new Error("AGENT_GRAPH_ENCRYPTION_KEY must be 32 bytes, base64url-encoded");
    }
    master = key;
  } else if (process.env.NODE_ENV === "production") {
    throw new Error("AGENT_GRAPH_ENCRYPTION_KEY must be set in production");
  } else {
    console.warn("AGENT_GRAPH_ENCRYPTION_KEY isn't set; using an insecure development key.");
    master = Buffer.alloc(KEY_BYTES, 0x5a);
  }
  return master;
}

// Deriving a key is cheap, but polls re-read the same logs, so keep a few.
const logKeys = new Map<string, Buffer>();
const LOG_KEY_CACHE_LIMIT = 1000;

function logKey(id: string): Buffer {
  let key = logKeys.get(id);
  if (!key) {
    key = Buffer.from(hkdfSync("sha256", masterKey(), "agent-graph", `log:${id}`, KEY_BYTES));
    if (logKeys.size >= LOG_KEY_CACHE_LIMIT) logKeys.delete(logKeys.keys().next().value!);
    logKeys.set(id, key);
  }
  return key;
}

// Keys for other purposes, derived from the master key like the logs' own.
const subKeys = new Map<string, Buffer>();

function subKey(purpose: "storage-id" | "meta"): Buffer {
  let key = subKeys.get(purpose);
  if (!key) {
    key = Buffer.from(hkdfSync("sha256", masterKey(), "agent-graph", purpose, KEY_BYTES));
    subKeys.set(purpose, key);
  }
  return key;
}

/** Where log `id` is stored: an HMAC of it, from which the id can't be had. */
export function storageId(id: string): string {
  return createHmac("sha256", subKey("storage-id")).update(id).digest("base64url");
}

/** The MAC of log `id`'s metadata, which is stored with it. */
export function metaTag(id: string, meta: { source: string; pw?: string }): string {
  return createHmac("sha256", subKey("meta"))
    .update(JSON.stringify(["meta/1", id, meta.source, meta.pw || null]))
    .digest("base64url");
}

/** What a chunk is bound to: its log and its place in it. */
function associatedData(id: string, chunk: string): Buffer {
  return Buffer.from(`agent-graph:${id}/${chunk}`);
}

/** Encrypts the text of chunk `chunk` (its document id) of log `id`. */
export function encryptChunk(id: string, chunk: string, text: string): Buffer {
  const iv = randomBytes(IV_BYTES);
  const cipher = createCipheriv("aes-256-gcm", logKey(id), iv);
  cipher.setAAD(associatedData(id, chunk));
  const body = Buffer.concat([cipher.update(text, "utf8"), cipher.final()]);
  return Buffer.concat([Buffer.of(VERSION), iv, body, cipher.getAuthTag()]);
}

/** Decrypts a chunk. Throws if it was altered, moved, or the key is wrong. */
export function decryptChunk(id: string, chunk: string, stored: Uint8Array): string {
  const data = Buffer.from(stored);
  if (data.length < 1 + IV_BYTES + TAG_BYTES || data[0] !== VERSION) {
    // Not the log's id: it's what lets people read the log, and errors get logged.
    throw new Error(`chunk ${chunk} isn't in a format this site can read`);
  }
  const iv = data.subarray(1, 1 + IV_BYTES);
  const tag = data.subarray(data.length - TAG_BYTES);
  const body = data.subarray(1 + IV_BYTES, data.length - TAG_BYTES);
  const decipher = createDecipheriv("aes-256-gcm", logKey(id), iv);
  decipher.setAAD(associatedData(id, chunk));
  decipher.setAuthTag(tag);
  return Buffer.concat([decipher.update(body), decipher.final()]).toString("utf8");
}
