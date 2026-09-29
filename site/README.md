# Agent Graph site

The site behind `https://agentgraph.chofter.com`: a pastebin for Agent Graph logs. You paste or upload a log, or stream one live with `agent-graph watch-remote`, and get a link to a viewer, kept until the log has had no new events for a week. The viewer is the same one `agent-graph view` serves locally, and anyone with the link can step through the log. Logs can be password protected.

It's a Next.js app with Firestore for storage. The viewer's graph logic isn't reimplemented here: it's the Rust reducer, compiled to WebAssembly and run in the browser.

## How it fits together

| Path                                                                         | What it is                                                                                                                                               |
| ---------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `app/page.tsx`, `app/uploader.tsx`                                           | Home page: paste, upload, or instructions for `watch-remote`                                                                                             |
| `app/l/[id]/page.tsx`                                                        | A shared log: the viewer shell, or a password prompt                                                                                                     |
| `app/docs/page.tsx`, `lib/help-render.tsx`                                   | The docs: the CLI's help text, rendered from `lib/cli-help.json`, which `dev`, `build` and `typecheck` copy from `../docs/cli-help.json` (not committed) |
| `app/api/logs/…`                                                             | The API (below)                                                                                                                                          |
| `lib/store.ts`                                                               | Firestore reads and writes                                                                                                                               |
| `lib/crypto.ts`                                                              | Ids, write keys, viewer cookies, password hashes                                                                                                         |
| `lib/encryption.ts`                                                          | Encrypts log chunks before they're stored                                                                                                                |
| `public/viewer/site-source.js`, `site-worker.js`                             | Feeds the viewer from the API instead of a local server; the worker reads the log and runs the WebAssembly, off the page's main thread (hand-written)    |
| `public/viewer/app.js`, `app.css`, `agent_graph.wasm`, `lib/viewer-shell.ts` | **Generated** from the Rust crate by `scripts/sync-viewer.mjs`; don't edit                                                                               |

The generated files are committed, so building the site doesn't need Rust. After changing the viewer (`../src/view/assets/`) or the reducer, regenerate them. This needs Rust and `rustup target add wasm32-unknown-unknown`:

```bash
npm run build-wasm
```

(`npm run dev` and `npm run build` re-copy the viewer files automatically when `../src` is present; only the WebAssembly needs this step. It records what it was built from in `agent_graph.wasm.sources` (the crate's files the build compiled, the crates it used with their versions and features, and the build profile), and CI fails if any of that has changed since: `node scripts/sync-viewer.mjs --check-wasm`. Paths on the build machine are left out of the binary.)

## Accounts

A live share (`agent-graph watch-remote`) belongs to an account, and only its owner can see it, at `/watch`. Pasted and uploaded logs stay anonymous, viewable by anyone with the link (and the password, if there is one).

- **Logging in** (`app/login`) is Firebase Authentication in the browser: Google, or an email and password (with account creation and password reset). The browser talks to Firebase directly, so signing in costs the site nothing but one request, and Firebase limits password guesses itself. The sign-in is traded for the site's own session cookie (`__session`, HttpOnly, two weeks: `lib/auth.ts`), which the server checks; the browser's Firebase sign-in is then dropped. A second cookie, holding nothing, tells the header to show Account rather than Log in.
- **The CLI** logs in through the browser too, as an OAuth client on the same computer would, with PKCE (`lib/accounts.ts`, `src/account.rs`): it opens `/login?cli=<port>&state=…&challenge=…`, and once logged in (and asked to connect it) the page sends the browser to `http://127.0.0.1:<port>/callback` with a one-time code, which the CLI trades, with its secret, for a token of its own. It sends the token to make a share (`Authorization: Bearer`), which is then its account's.
- **What's stored:** `users/{uid}` is made when an account first logs in to the site, and holds its email, when it was made (`createdAt`), last logged in (`lastLoginAt`: in a browser, or the CLI on a computer) and last shared live (`lastWatchAt`), and that latest share's id, sealed (the database never holds a log's id in the clear); `cli-tokens/{sha256}` each computer's login, and `cli-codes/{sha256}` codes not yet traded, by their SHA-256, never as they are. A share's owner is in its metadata, and bound into its tag.
- **Paying** (`lib/billing.ts`, `lib/stripe.ts`), once Stripe's set up (`STRIPE.md`); without it, sharing live is free, and every account is `active`:
  - An account is made `unpaid`, and can share live for its first `FREE_TRIAL_DAYS` days anyway.
  - After that, only once it subscribes (`active`), monthly or yearly, on its account page, with Stripe Checkout. `STRIPE_MODE` says whether that's Stripe's test mode or live.
  - Starting or carrying on with a share checks where the account stands (`402` if it has to subscribe), and stamps the log with when that runs out (`until`). Appends after that are refused with `402`, with no extra database read, and the CLI starts the share again, which checks again.
  - When a share is refused, `watch-remote` opens `/account` and asks `GET /api/cli/account` every 5 s until the account can share.
  - Stripe's webhook (`/api/stripe/webhook`) keeps `users/{uid}` up to date: `status`, `stripeCustomer`, and `subscription` (its Stripe status, its plan, and when its paid period ends).
- **Pages:** `/login`, `/account` (who's logged in, their subscription, the latest share, the computers logged in, each with a way to log it out) and `/watch` (the latest share, in the viewer; logged out, it goes to `/login?next=/watch`).

## The API

All bodies are raw JSON Lines, at most 512 KB per request, cut at line boundaries.

| Request                                            | What it does                                                                                                                                                                                                                                                      |
| -------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `POST /api/logs`                                   | Creates a log from the first chunk. Optional headers: `X-Agent-Graph-Source: watch\|paste\|upload`, `X-Agent-Graph-Password: <base64url of the UTF-8 password>` (at most 1024 bytes of UTF-8, the longest unlocking checks). Replies `201 {id, url, writeToken}`. |
| `POST /api/logs/{id}/append?offset=<bytes so far>` | Appends a chunk. Requires `Authorization: Bearer <writeToken>`. Replies `204`, also for the same bytes again (a retry); `409` if other bytes are already stored at that offset.                                                                                   |
| `POST /api/logs/{id}/trim?before=<offset>`         | Deletes the chunks that start before `offset`. Requires the `writeToken`. A live share keeps only its last two keyframes' worth: see `docs/design.md` §9.                                                                                                         |
| `GET /api/logs/{id}/content?after=<chunk key>`     | Chunks after `after`, joined, up to about 2 MB, stopping at a gap (a trim). `X-First-Chunk` is where the text starts (its offset); `X-Last-Chunk` is the next cursor; `X-More: 1` means fetch again now. Protected logs need the unlock cookie.                   |
| `GET /api/cron/cleanup`                            | Deletes logs that have had no event for a week (`lib/cleanup.ts`). Run daily by Vercel Cron; requires `Authorization: Bearer <CRON_SECRET>`. Replies `{checked, deleted, done}`.                                                                                  |
| `POST /api/logs/{id}/unlock`                       | `{"password": "…"}`. Sets an HttpOnly cookie for this log.                                                                                                                                                                                                        |

**Only the creator can add to a log.**

- The `writeToken` from the create call is required on every append.
- It's an HMAC of the log id under `AGENT_GRAPH_SECRET`, so it can't be forged or guessed, and it's never stored anywhere.
- The CLI and the upload page hold it only in memory.
- There's no endpoint that changes or deletes a log, but for trimming its start, which also needs the `writeToken`.

**Appending is kept cheap:**

- The size is checked from the header before the body is read, and as it's read: one sent without a `Content-Length` is read only as far as the limit. (So is every other body: unlocking's, at most 8 KB.)
- The key is checked by recomputing an HMAC: no database read.
- The body is never parsed.
- Storage is a single write of a new document, in a transaction with two reads: the log's metadata, that the log is still there (one not in use is deleted: an append to it is refused with 410, and the sharer stops), and the chunk it follows on from (each records where it ends): a chunk must start where one ends, or be the log's first, else it's refused (409). Each chunk is its own document, `logs/{sid}/chunks/{offset}` (`sid` is an HMAC of the log's id, below), zero-padded so ids sort in order, so the cost doesn't grow with the log. A chunk never changes once stored: a retry of the same bytes is accepted, and different bytes at a stored offset are refused (409).
- **A log stores at most 64 MiB.** Chunks never overlap, so that's the span from its first chunk to its last: an append more than 64 MiB past where the log now starts is refused with `413`. A live share that trims its start can go on for good.

**Reading:**

- Each viewer polls every 3 s while events are arriving, backing off to 15 s when quiet or when the tab is hidden.
- A log's metadata never changes, so each server instance caches it.

Passwords are hashed with scrypt. Wrong guesses are limited, every 15 minutes: 5 at a log from one address, 20 at a log from anywhere, and 30 from one address across logs (counted in Firestore, so across every server instance; a right password doesn't count). Past a limit, unlocking answers `429` until the window ends, even with the right password.

- **Every scrypt run is limited too,** since each costs the server about 50 ms, right passwords included: every 15 minutes, 200 passwords checked at a log from one address, and 2,000 runs from one address in all (passwords checked at any log, and logs created with a password). Past the first, unlocking that log from that address answers `429` until the window ends, so one log's viewers behind a shared address (or one viewer unlocking it again and again) hold back only that log there. Past the second, so do unlocking any log and creating one with a password from that address; creating one without a password doesn't.
- **What that costs viewers:** one address can only hold back itself and anyone sharing it (an office, or a mobile carrier's shared address), but someone guessing from four or more addresses can keep a log's new viewers waiting for as long as they keep at it. Viewers who have already unlocked it aren't affected.
- **What it allows guessers:** about 2,000 guesses a day at a log, for as long as it exists. Choose a password that wouldn't fall to that: not a word, a name or a date.
- **Addresses** are the ones Vercel reports (`X-Real-IP`), IPv6 counted by its /64. Behind another proxy, check it sets that header, or the per-address limits can be dodged (the per-log one holds regardless).

Firestore's security rules (`firestore.rules`) deny all direct access; only the server, using the Admin SDK, touches the data.

**Logs are encrypted at rest.** Google already encrypts Firestore's disks. On top of that, the site encrypts every chunk before storing it, so the database holds only ciphertext. Anyone who can read Firestore itself (the console, exports and backups, a leaked service-account key) sees nothing readable.

- **Cipher:** AES-256-GCM, which also detects any change to stored data.
- **Per-log keys:** each log's key is derived (HKDF) from the master key `AGENT_GRAPH_ENCRYPTION_KEY` and the log's id.
- **Binding:** each chunk is bound to its log and position, so chunks can't be swapped or reordered undetected.
- **The links aren't stored.** A log's id is its link, so logs are stored under an HMAC of it (keyed from the master key), which can't be turned back into the link. Someone with the database can't open the logs through the site. (Except from an export or backup made before logs were stored this way: see Deploy.)
- **The metadata is authenticated.** Each log's metadata carries a MAC bound to its id, so a password removed from it, or another log's metadata copied over it, is refused.
- **Cost:** a fraction of a millisecond per chunk.
- **Not encrypted:** the metadata (when a log was created, how it was shared, the password _hash_, how many bytes it stores) and the chunk ids and lengths, which reveal a log's size. (When a log was created isn't authenticated either; nothing depends on it.)
- **The server can still read logs.** It holds the key; this isn't end-to-end encryption.

## Develop

Requires Node 22, plus the Firebase CLI and Java for the emulators (Firestore, and Authentication for accounts).

```bash
npm install
```

This also sets up the repository's pre-commit hook (`../.githooks/pre-commit`), which formats the site's staged files with Prettier before each commit. `npm run format` formats everything; CI fails on code it would change (`npm run format:check`).

```bash
cp .env.example .env.local
```

In `.env.local`, uncomment the emulator lines and set `AGENT_GRAPH_SECRET` and `AGENT_GRAPH_ENCRYPTION_KEY` (both generated as shown in the file). Then start the emulators (logging in then works with any email and password, and the emulator stands in for Google):

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
2. **Service account:** in Firebase, go to Project settings → Service accounts and generate a new private key. It needs the **Firebase Authentication Admin** role as well as Firestore's (the site makes and checks session cookies with it).
3. **Accounts:** in Firebase Authentication, enable the **Email/Password** and **Google** sign-in providers, and add `agentgraph.chofter.com` to **Settings → Authorized domains**. Add a web app (Project settings → Your apps) for its settings, below.
4. **Vercel:** import the repository and set **Root Directory** to `site`.
5. **Environment variables:** set these in Vercel.
   - `AGENT_GRAPH_SECRET`: 32+ random bytes. Generate one with:
     ```bash
     node -e "console.log(require('crypto').randomBytes(32).toString('base64url'))"
     ```
     Changing it invalidates every write key and viewer cookie.
   - `AGENT_GRAPH_ENCRYPTION_KEY`: another 32 random bytes, generated the same way. **Back it up and never change it**: without it, stored logs can't be decrypted.
   - `FIREBASE_SERVICE_ACCOUNT`: the service account's JSON key, on one line.
   - `NEXT_PUBLIC_SITE_URL`: `https://agentgraph.chofter.com`
   - `CRON_SECRET`: 32+ random bytes, generated the same way. Vercel Cron sends it to `/api/cron/cleanup` (see `vercel.json`), the daily deletion of logs with no event for a week, which does nothing without it.
   - `NEXT_PUBLIC_FIREBASE_API_KEY`, `NEXT_PUBLIC_FIREBASE_AUTH_DOMAIN`, `NEXT_PUBLIC_FIREBASE_PROJECT_ID`, `NEXT_PUBLIC_FIREBASE_APP_ID`: the web app's settings, from step 3. They're public by design (they identify the project; what's secret stays on the server), and built into the pages: redeploy after changing them.
   - Optionally, to charge for sharing live: `STRIPE_MODE` (`test` or `production`), that mode's keys and prices (`STRIPE_SECRET_KEY`, `STRIPE_WEBHOOK_SECRET`, `STRIPE_PRICE_MONTHLY` and `STRIPE_PRICE_YEARLY`, or the same with `STRIPE_TEST_` in test mode), `STRIPE_MONTHLY_LABEL`, `STRIPE_YEARLY_LABEL`, and `FREE_TRIAL_DAYS`. Setting up Stripe for them, step by step: `STRIPE.md`.
6. **Domain:** add `agentgraph.chofter.com` in Vercel, and a `CNAME` record for `agentgraph` pointing at `cname.vercel-dns.com`.
7. **Optional:** add Firestore TTL policies on the `expireAt` field of the `unlock-attempts` collection, so counts of password guesses and scrypt runs are cleared away once their window is over, and of `cli-codes`, for logins the CLI never finished (an expired code is refused either way).
8. **Logs from before storage ids.** Logs created before logs were stored under an HMAC of their id can't be found by the new site until they're copied. Run the migration with the production environment: the same `AGENT_GRAPH_ENCRYPTION_KEY`, and `FIREBASE_SERVICE_ACCOUNT`. It refuses to run without the key, prints the project, and checks that the key decrypts the logs before writing anything.
   1. Before deploying, copy the logs. The old site doesn't see the copies.
      ```bash
      npm run migrate:storage-ids
      ```
   2. Deploy, and straight away copy again, for what the old site stored in between: until then, logs it created since the first copy aren't found, and those it added to lack their latest events. (Someone who opened a log being shared live in those minutes may need to reload it.)
   3. Once the site works, delete the old copies. Each log's are deleted only once they're all copied.
      ```bash
      npm run migrate:storage-ids -- --delete-old
      ```

   Copies are marked until their old copy is deleted, so the deletion of logs not in use (`CRON_SECRET`, above) deletes the two together. Copies made by the migration before it marked them aren't: if you copied logs then and haven't done step 3 yet, copy them again (step 1's command) once the cron is deployed, which marks them.

   Rolling the site back past this deploy loses the logs created since (the old site can't find them), and after step 3, all of them. Exports, backups and point-in-time recovery (which keeps deleted documents for up to 7 days) from before step 3 still name every log by its link: delete them, or keep them as safe as the logs.

Not built yet:

- rate limiting on log creation (only hashing a new log's password is limited);
- a way to delete a log;
- rotating the encryption key. The stored format carries a version byte, so a second key can be added later, but existing logs can't be re-encrypted with it: that needs each log's id, which isn't stored. The same goes for anyone who gets the key and the database without the links: they can't decrypt anything.
