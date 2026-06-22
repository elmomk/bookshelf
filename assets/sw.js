// NOTE: scripts/deploy.sh rewrites this line with a unique build id on every
// deploy (auto cache-bust). 'bookclub-dev' is the value used by `dx serve`.
const CACHE_NAME = 'bookclub-dev';

// Book covers come from cross-origin hosts (Google Books / Open Library) and
// are content-immutable. They live in their own cache that is NOT version-
// busted on deploy, so the shelf keeps showing covers offline across releases.
const COVER_CACHE = 'bookclub-covers';

const PRECACHE_URLS = [
  '/',
  '/manifest.json',
  '/main.css',
  '/sw-register.js',
  '/icons/icon-192.png',
  '/icons/icon-512.png',
  '/icons/icon.svg',
  '/fonts/JetBrainsMono-Regular.ttf',
  '/fonts/JetBrainsMono-Medium.ttf',
  '/fonts/JetBrainsMono-Bold.ttf',
];

self.addEventListener('install', (event) => {
  event.waitUntil(
    caches.open(CACHE_NAME).then((cache) =>
      // Use individual put so a single 404 doesn't fail the whole install
      Promise.all(
        PRECACHE_URLS.map((url) =>
          fetch(url, { cache: 'reload' })
            .then((res) => res.ok ? cache.put(url, res) : null)
            .catch(() => null)
        )
      )
    )
  );
  self.skipWaiting();
});

self.addEventListener('activate', (event) => {
  event.waitUntil(
    caches.keys().then((names) =>
      Promise.all(
        names
          .filter((n) => n !== CACHE_NAME && n !== COVER_CACHE)
          .map((n) => caches.delete(n))
      )
    )
  );
  self.clients.claim();
});

// Hashed assets (WASM, JS with -dxh in filename) are immutable — content hash changes on rebuild
function isHashedAsset(url) {
  return url.includes('-dxh');
}

self.addEventListener('fetch', (event) => {
  if (event.request.method !== 'GET') return;

  const url = new URL(event.request.url);

  // Book covers (cross-origin images from Google Books / Open Library):
  // cache-first into the persistent cover cache so the shelf and book pages
  // render covers offline. These are no-cors `opaque` responses (status 0),
  // so we can't inspect them — cache any fetch that doesn't throw.
  if (event.request.destination === 'image' && url.origin !== self.location.origin) {
    event.respondWith((async () => {
      const cache = await caches.open(COVER_CACHE);
      const cached = await cache.match(event.request);
      if (cached) return cached;
      try {
        const response = await fetch(event.request);
        if (response && (response.ok || response.type === 'opaque')) {
          cache.put(event.request, response.clone());
        }
        return response;
      } catch {
        return cached || Response.error();
      }
    })());
    return;
  }

  // Skip server function calls and any non-same-origin requests
  if (url.origin !== self.location.origin) return;
  if (url.pathname.startsWith('/api/')) return;

  // Navigation requests (HTML): stale-while-revalidate so app boots offline.
  // Falls back to the precached '/' index (SPA shell) if the specific path isn't cached.
  if (event.request.mode === 'navigate') {
    event.respondWith((async () => {
      const cache = await caches.open(CACHE_NAME);
      const cached = await cache.match(event.request);
      const networkPromise = fetch(event.request)
        .then((response) => {
          if (response.ok) cache.put(event.request, response.clone());
          return response;
        })
        .catch(() => null);
      if (cached) return cached;
      const fresh = await networkPromise;
      if (fresh) return fresh;
      const shell = await cache.match('/');
      return shell || new Response('Offline', { status: 503, headers: { 'Content-Type': 'text/plain' } });
    })());
    return;
  }

  // Hashed assets: cache-first (immutable)
  if (isHashedAsset(event.request.url)) {
    event.respondWith(
      caches.match(event.request).then((cached) => {
        if (cached) return cached;
        return fetch(event.request).then((response) => {
          if (response.ok) {
            const clone = response.clone();
            caches.open(CACHE_NAME).then((cache) => cache.put(event.request, clone));
          }
          return response;
        });
      })
    );
    return;
  }

  // Other GET assets (CSS, fonts, icons, manifest): stale-while-revalidate
  event.respondWith(
    caches.match(event.request).then((cached) => {
      const fetched = fetch(event.request).then((response) => {
        if (response.ok) {
          const clone = response.clone();
          caches.open(CACHE_NAME).then((cache) => cache.put(event.request, clone));
        }
        return response;
      }).catch(() => cached);

      return cached || fetched;
    })
  );
});

// Push notification support
self.addEventListener('push', (event) => {
  if (!event.data) return;

  let data;
  try {
    data = event.data.json();
  } catch {
    data = { title: 'Life Manager', body: event.data.text() };
  }

  const options = {
    body: data.body || '',
    icon: '/icons/icon-192.png',
    badge: '/icons/icon-192.png',
    tag: data.tag || 'default',
    data: { url: data.url || '/' }
  };

  event.waitUntil(self.registration.showNotification(data.title || 'Life Manager', options));
});

self.addEventListener('notificationclick', (event) => {
  event.notification.close();
  const url = event.notification.data?.url || '/';
  event.waitUntil(
    clients.matchAll({ type: 'window', includeUncontrolled: true }).then((windowClients) => {
      for (const client of windowClients) {
        if (client.url.includes(self.location.origin)) {
          client.focus();
          client.navigate(url);
          return;
        }
      }
      return clients.openWindow(url);
    })
  );
});
