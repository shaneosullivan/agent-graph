// The site's service worker, so it can be installed as an app (registered
// by app/app-updates.tsx). It caches nothing: every request goes to the
// network as it would without it, so it can never serve anything stale.
// Its one addition is a page that says so when a page can't be loaded
// because there's no connection.
//
// It's always fetched fresh (next.config.ts: no-cache), and a new one takes
// over at once, from every open page.

self.addEventListener("install", () => {
  self.skipWaiting();
});

self.addEventListener("activate", event => {
  event.waitUntil(
    (async () => {
      // Nothing's kept: any cache something left behind is emptied.
      const keys = await caches.keys();
      await Promise.all(keys.map(key => caches.delete(key)));
      await self.clients.claim();
    })(),
  );
});

const OFFLINE = `<!doctype html>
<html lang="en">
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Offline · Agent Graph</title>
<style>
  :root { color-scheme: light dark; }
  body { margin: 0; min-height: 100vh; display: grid; place-items: center;
    font: 16px/1.5 system-ui, sans-serif; background: #f6f6f3; color: #1d1d1f; }
  @media (prefers-color-scheme: dark) { body { background: #131315; color: #ececef; } }
  main { max-width: 26rem; padding: 24px; text-align: center; }
  button { font: inherit; padding: 8px 16px; border-radius: 8px; border: 1px solid #8884;
    background: transparent; color: inherit; cursor: pointer; }
</style>
<main>
  <h1>You're offline</h1>
  <p>Agent Graph needs a connection. It'll load once you're back online.</p>
  <button onclick="location.reload()">Try again</button>
</main>
<script>addEventListener("online", () => location.reload());</script>
</html>`;

self.addEventListener("fetch", event => {
  // Only pages: everything else is left to the browser, untouched.
  if (event.request.mode !== "navigate") {
    return;
  }
  event.respondWith(
    fetch(event.request).catch(
      () =>
        new Response(OFFLINE, {
          status: 503,
          headers: {
            "Content-Type": "text/html; charset=utf-8",
            "Cache-Control": "no-store",
          },
        }),
    ),
  );
});
