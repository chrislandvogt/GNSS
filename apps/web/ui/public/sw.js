// Service worker: caches this version of the app so it works offline
// (ARCHITECTURE.md section 14.3). The build replaces the placeholder below
// with the list of files and a content-derived version.
const MANIFEST = "__PRECACHE_MANIFEST__"
const CACHE = `gnss-app-${MANIFEST.version}`

self.addEventListener('install', (event) => {
  event.waitUntil(
    caches.open(CACHE).then((cache) => cache.addAll(['./', ...MANIFEST.files])),
  )
  // No skipWaiting(): a new version takes effect on reload, never in the
  // middle of a job.
})

self.addEventListener('activate', (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((keys) =>
        Promise.all(
          keys.filter((k) => k.startsWith('gnss-app-') && k !== CACHE).map((k) => caches.delete(k)),
        ),
      ),
  )
})

self.addEventListener('fetch', (event) => {
  const request = event.request
  if (request.method !== 'GET' || new URL(request.url).origin !== self.location.origin) return
  event.respondWith(
    caches
      .match(request, { cacheName: CACHE, ignoreSearch: true })
      .then((cached) => cached ?? fetch(request)),
  )
})
