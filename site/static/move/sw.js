// bible.rasmic.xyz serves this as its /service-worker.js (scripts/vercel-routes.js): it replaces
// the reader's worker there, empties its caches and unregisters, so every visit reaches move.js.
self.addEventListener('install', () => self.skipWaiting());
self.addEventListener('activate', (event) => {
	event.waitUntil(
		caches
			.keys()
			.then((keys) => Promise.all(keys.map((k) => caches.delete(k))))
			.then(() => self.registration.unregister())
	);
});
