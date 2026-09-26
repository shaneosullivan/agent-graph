// Moves logs stored the old way (under their ids, which are the links that
// open them) to where they're kept now: under an HMAC of the id, with their
// metadata's tag. It needs the site's production environment:
// AGENT_GRAPH_ENCRYPTION_KEY, and FIREBASE_SERVICE_ACCOUNT (or application
// default credentials).
//
//   npm run migrate:storage-ids                  copies; deletes nothing
//   npm run migrate:storage-ids -- --delete-old  deletes the old copies
//
// Copying is safe to run before deploying (the old site doesn't see the
// copies), again after, and as often as needed. --delete-old deletes a log's
// old documents only once every one of them has a checked copy.
//
// The key is checked before anything is written, against the old logs'
// chunks, and again for each log. With the wrong key, the copies would be
// where the site can't find them, and nothing could find them again: the
// ids are only in the old documents' names.

import { register } from "node:module";

register("./resolve-ts.mjs", import.meta.url);

const deleteOld = process.argv.includes("--delete-old");

if (!process.env.AGENT_GRAPH_ENCRYPTION_KEY && !process.env.FIRESTORE_EMULATOR_HOST) {
  console.error(
    "Set AGENT_GRAPH_ENCRYPTION_KEY to the site's key (and FIREBASE_SERVICE_ACCOUNT): " +
      "with any other key, the logs would be moved where the site can't find them.",
  );
  process.exit(2);
}

const { getApps } = await import("firebase-admin/app");
const { FieldPath } = await import("firebase-admin/firestore");
const { ID_PATTERN } = await import("../lib/config.ts");
const { decryptChunk, metaTag, storageId } = await import("../lib/encryption.ts");
const { firestore } = await import("../lib/firebase.ts");

const db = firestore();
const logs = db.collection("logs");
console.log(`Project: ${getApps()[0]?.options.projectId ?? "the environment's default"}`);

/** A log's chunks, a page at a time (they can be 512 KB each). */
async function* pages(log) {
  const query = log.collection("chunks").orderBy(FieldPath.documentId()).limit(16);
  let last = null;
  for (;;) {
    const snap = await (last ? query.startAfter(last) : query).get();
    if (snap.empty) return;
    yield snap.docs;
    if (snap.size < 16) return;
    last = snap.docs[snap.size - 1].id;
  }
}

/** The metadata as it's stored now, from the old document; throws if it's incomplete. */
function newMeta(id, data) {
  const { source, pw, createdAt } = data;
  if (typeof source !== "string" || !createdAt) throw new Error("its metadata has no source or creation time");
  return { source, ...(pw ? { pw } : {}), createdAt, mac: metaTag(id, { source, pw }) };
}

/**
 * Writes with a BulkWriter, a page at a time: `write` queues writes, and
 * each page's must all succeed before the next (a failed write rejects only
 * its own promise, never `flush`).
 */
async function inPages(write) {
  const writer = db.bulkWriter();
  try {
    await write(async (queue) => {
      // Queued first, then waited on with the flush: a write that fails
      // while flush() waits on another's retry is still caught here.
      const writes = queue(writer);
      await Promise.all([...writes, writer.flush()]);
    });
  } finally {
    await writer.close();
  }
}

/** Copies log `id`: its chunks first, then its metadata, so it only shows once complete. */
async function copy(id, old, meta) {
  const target = logs.doc(storageId(id));
  await inPages(async (page) => {
    let checked = false;
    for await (const chunks of pages(old)) {
      if (!checked) {
        // A damaged log: nothing of it is written.
        decryptChunk(id, chunks[0].id, chunks[0].get("e"));
        checked = true;
      }
      await page((writer) => chunks.map((chunk) => writer.set(target.collection("chunks").doc(chunk.id), chunk.data())));
    }
    if (meta.exists) await page((writer) => [writer.set(target, newMeta(id, meta.data()))]);
  });
}

/** Deletes log `id`'s old documents, each once its copy is checked. */
async function removeOld(id, old, meta) {
  const target = logs.doc(storageId(id));
  if (meta.exists) {
    const copied = await target.get();
    const expected = newMeta(id, meta.data());
    if (!copied.exists || copied.get("mac") !== expected.mac) throw new Error("its metadata hasn't been copied");
  }
  await inPages(async (page) => {
    for await (const chunks of pages(old)) {
      const copies = await db.getAll(...chunks.map((chunk) => target.collection("chunks").doc(chunk.id)));
      chunks.forEach((chunk, i) => {
        // Compared as text: a chunk the new site stored itself (a retry) has
        // other ciphertext for the same text.
        const copy = copies[i];
        if (!copy.exists || decryptChunk(id, chunk.id, copy.get("e")) !== decryptChunk(id, chunk.id, chunk.get("e"))) {
          throw new Error(`chunk ${chunk.id} hasn't been copied`);
        }
      });
      await page((writer) => chunks.map((chunk) => writer.delete(chunk.ref)));
    }
    if (meta.exists) await page((writer) => [writer.delete(old)]);
  });
}

/** Whether log `id`'s copy has a chunk (the new site stored it) that decrypts: proof of the key. */
async function copyDecrypts(id) {
  const first = await logs.doc(storageId(id)).collection("chunks").limit(1).get();
  if (first.empty) return false;
  try {
    decryptChunk(id, first.docs[0].id, first.docs[0].get("e"));
    return true;
  } catch {
    return false;
  }
}

// Old documents are named by the id (12 characters); new ones never are.
// The most recently created go first.
const old = (await logs.listDocuments()).filter((doc) => ID_PATTERN.test(doc.id));
const metas = [];
for (let i = 0; i < old.length; i += 500) metas.push(...(await db.getAll(...old.slice(i, i + 500))));
const order = metas
  .map((meta, i) => ({ doc: old[i], meta }))
  .sort((a, b) => (b.meta.get("createdAt")?.toMillis() ?? 0) - (a.meta.get("createdAt")?.toMillis() ?? 0));

/**
 * Whether the key decrypts the old logs' chunks: stops if it decrypts none of
 * the first few logs' (a damaged log can't stop it alone), and says false if
 * there are no chunks to check it against.
 */
async function keyWorks() {
  const tried = [];
  for (const { doc } of order) {
    const first = await doc.collection("chunks").orderBy(FieldPath.documentId()).limit(1).get();
    if (first.empty) continue;
    try {
      decryptChunk(doc.id, first.docs[0].id, first.docs[0].get("e"));
      return true;
    } catch {
      tried.push(doc.id);
      if (tried.length === 5) break;
    }
  }
  if (tried.length) {
    console.error(
      `No old log checked (${tried.map((id) => `logs/${id}`).join(", ")}) decrypts with this ` +
        "AGENT_GRAPH_ENCRYPTION_KEY: either it isn't the site's key, or those logs are damaged. Nothing was changed.",
    );
    process.exit(1);
  }
  return false;
}

const keyChecked = await keyWorks();
let done = 0;
const failed = [];
for (const { doc, meta } of order) {
  try {
    if (deleteOld) {
      // A log without chunks can't prove the key itself; another log, or its
      // own copy's chunks (stored by the new site), must.
      const empty = (await doc.collection("chunks").limit(1).get()).empty;
      if (empty && !keyChecked && !(await copyDecrypts(doc.id))) {
        throw new Error("nothing could check the key, so it's kept (if the log is empty, delete it yourself)");
      }
      await removeOld(doc.id, doc, meta);
    } else {
      await copy(doc.id, doc, meta);
    }
    done++;
  } catch (err) {
    failed.push(doc.id);
    // Named by its id, which is its old document's name: it's the operator's database.
    console.error(`logs/${doc.id}: ${err instanceof Error ? err.message : err}`);
  }
}
const logsWord = (n) => `${n} log${n === 1 ? "" : "s"}`;
console.log(deleteOld ? `Deleted the old copies of ${logsWord(done)}.` : `Copied ${logsWord(done)}.`);
if (failed.length) {
  console.error(`${logsWord(failed.length)} failed (above), and were left as they were.`);
  process.exit(1);
}
