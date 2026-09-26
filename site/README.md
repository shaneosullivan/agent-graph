# Agent Graph site

The site behind `https://agentgraph.chofter.com`: a pastebin for Agent Graph logs. You paste or upload a log, or stream one live with `agent-graph watch-remote`, and get a permanent link to a viewer. The viewer is the same one `agent-graph view` serves locally, and anyone with the link can step through the log. Logs can be password protected.

It's a Next.js app with Firestore for storage. The viewer's graph logic isn't reimplemented here: it's the Rust reducer, compiled to WebAssembly and run in the browser.

## How it fits together

| Path | What it is |
|---|---|
| `app/page.tsx`, `app/uploader.tsx` | Home page: paste, upload, or instructions for `watch-remote` |
| `app/l/[id]/page.tsx` | A shared log: the viewer shell, or a password prompt |
| `app/docs/page.tsx`, `lib/help-render.tsx` | The docs: the CLI's help text, rendered from `lib/cli-help.json`, which `dev`, `build` and `typecheck` copy from `../docs/cli-help.json` (not committed) |
| `app/api/logs/…` | The API (below) |
| `lib/store.ts` | Firestore reads and writes |
| `lib/crypto.ts` | Ids, write keys, viewer cookies, password hashes |
| `lib/encryption.ts` | Encrypts log chunks before they're stored |
| `public/viewer/site-source.js` | Feeds the viewer from the API instead of a local server (hand-written) |
| `public/viewer/app.js`, `app.css`, `agent_graph.wasm`, `lib/viewer-shell.ts` | **Generated** from the Rust crate by `scripts/sync-viewer.mjs`; don't edit |

The generated files are committed, so building the site doesn't need Rust. After changing the viewer (`../src/view/assets/`) or the reducer, regenerate them. This needs Rust and `rustup target add wasm32-unknown-unknown`:

```bash
npm run build-wasm
```

(`npm run dev` and `npm run build` re-copy the viewer files automatically when `../src` is present; only the WebAssembly needs this step.)

## The API

All bodies are raw JSON Lines, at most 512 KB per request, cut at line boundaries.

| Request | What it does |
|---|---|
| `POST /api/logs` | Creates a log from the first chunk. Optional headers: `X-Agent-Graph-Source: watch\|paste\|upload`, `X-Agent-Graph-Password: <base64url of the UTF-8 password>`. Replies `201 {id, url, writeToken}`. |
| `POST /api/logs/{id}/append?offset=<bytes so far>` | Appends a chunk. Requires `Authorization: Bearer <writeToken>`. Replies `204`, also for the same bytes again (a retry); `409` if other bytes are already stored at that offset. |
| `GET /api/logs/{id}/content?after=<chunk key>` | Chunks after `after`, joined, up to about 2 MB. `X-Last-Chunk` is the next cursor; `X-More: 1` means fetch again now. Protected logs need the unlock cookie. |
| `POST /api/logs/{id}/unlock` | `{"password": "…"}`. Sets an HttpOnly cookie for this log. |

**Only the creator can add to a log.**
- The `writeToken` from the create call is required on every append.
- It's an HMAC of the log id under `AGENT_GRAPH_SECRET`, so it can't be forged or guessed, and it's never stored anywhere.
- The CLI and the upload page hold it only in memory.
- There's no endpoint that changes or deletes a log.

**Appending is kept cheap:**
- The size is checked from the header before the body is read.
- The key is checked by recomputing an HMAC: no database read.
- The body is never parsed.
- Storage is a single write of a new document. Each chunk is its own document, `logs/{id}/chunks/{offset}`, zero-padded so ids sort in order, so the cost doesn't grow with the log. A chunk never changes once stored: a retry of the same bytes is accepted, and different bytes at a stored offset are refused (409).

**Reading:**
- Each viewer polls every 3 s while events are arriving, backing off to 15 s when quiet or when the tab is hidden.
- A log's metadata never changes, so each server instance caches it.

Passwords are hashed with scrypt. Firestore's security rules (`firestore.rules`) deny all direct access; only the server, using the Admin SDK, touches the data.

**Logs are encrypted at rest.** Google already encrypts Firestore's disks. On top of that, the site encrypts every chunk before storing it, so the database holds only ciphertext. Anyone who can read Firestore itself (the console, exports and backups, a leaked service-account key) sees nothing readable.
- **Cipher:** AES-256-GCM, which also detects any change to stored data.
- **Per-log keys:** each log's key is derived (HKDF) from the master key `AGENT_GRAPH_ENCRYPTION_KEY` and the log's id.
- **Binding:** each chunk is bound to its log and position, so chunks can't be swapped or reordered undetected.
- **Cost:** a fraction of a millisecond per chunk, and appends still make no database reads.
- **Not encrypted:** the metadata (when a log was created, how it was shared, the password *hash*) and the chunk ids, which reveal a log's size.
- **The server can still read logs.** It holds the key; this isn't end-to-end encryption.

## Develop

Requires Node 22, plus the Firebase CLI and Java for the Firestore emulator.

```bash
npm install
```

```bash
cp .env.example .env.local
```

In `.env.local`, uncomment the two emulator lines and set `AGENT_GRAPH_SECRET` and `AGENT_GRAPH_ENCRYPTION_KEY` (both generated as shown in the file). Then start the emulator:

```bash
npm run emulators
```

And, in another terminal, the site:

```bash
npm run dev
```

To stream your own agents to it:

```bash
agent-graph watch-remote --url=http://localhost:3000
```

The encryption has unit tests:

```bash
npm run test:unit
```

The API tests run against any running copy of the site:

```bash
BASE_URL=http://localhost:3000 npm run test:api
```

Or, as CI does, against a production build with its own emulator, followed by `lib/store.ts`'s own tests (`npm run test:store`, which needs the emulator). Run `npm run build` first, and stop any running emulator:

```bash
npm run test:ci
```

## Deploy (Vercel)

1. **Firebase:** create a project, enable Firestore (Native mode), and deploy the deny-all rules:
   ```bash
   firebase deploy --only firestore:rules --project <project-id>
   ```
2. **Service account:** in Firebase, go to Project settings → Service accounts and generate a new private key.
3. **Vercel:** import the repository and set **Root Directory** to `site`.
4. **Environment variables:** set these in Vercel.
   - `AGENT_GRAPH_SECRET`: 32+ random bytes. Generate one with:
     ```bash
     node -e "console.log(require('crypto').randomBytes(32).toString('base64url'))"
     ```
     Changing it invalidates every write key and viewer cookie.
   - `AGENT_GRAPH_ENCRYPTION_KEY`: another 32 random bytes, generated the same way. **Back it up and never change it**: without it, stored logs can't be decrypted.
   - `FIREBASE_SERVICE_ACCOUNT`: the service account's JSON key, on one line.
   - `NEXT_PUBLIC_SITE_URL`: `https://agentgraph.chofter.com`
5. **Domain:** add `agentgraph.chofter.com` in Vercel, and a `CNAME` record for `agentgraph` pointing at `cname.vercel-dns.com`.

Not built yet:
- rate limiting on log creation;
- a way to delete a log;
- rotating the encryption key (the stored format carries a version byte, so a second key can be added later).
