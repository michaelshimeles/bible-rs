/**
 * Moves a reader's library from the old address (bible.rasmic.xyz) to simplymybible.com.
 *
 * A browser keeps each address's localStorage apart, so the library a reader built on the old
 * address stays there. The old address serves `out.html` for every path (scripts/vercel-routes.js).
 * That page reads the library and hands it to `in.html` on simplymybible.com in the address's
 * fragment, which browsers never send to a server. A library too big for one address goes in
 * pieces: `in.html` keeps each piece in sessionStorage and sends the reader back for the next.
 * `in.html` checks the pieces against the library's hash, merges it into what simplymybible.com
 * already holds (the newer of two edits to a verse wins, nothing is deleted), remembers the hash
 * so the same library never merges twice, and opens the page the reader asked for. The old
 * address keeps its copy; `bible.rasmic.xyz/#keep` downloads it as a JSON backup.
 *
 * Plain JavaScript with no imports, so the pages load it without the site's build.
 */

export const OLD = 'https://bible.rasmic.xyz';
export const NEW = 'https://simplymybible.com';

/** The reader's localStorage keys (lib/read/settings.ts, app.svelte.ts, imports.ts; lib/theme). */
export const KEYS = {
	settings: 'bible.read.settings',
	study: 'bible.read.study',
	backups: 'bible.read.backups',
	imported: 'bible.read.imported',
	theme: 'theme'
};
/** On simplymybible.com: the hashes of the libraries merged in. */
export const MOVED_KEY = 'bible.read.moved';
/** On simplymybible.com: what the reader says once it opens (lib/read/moved.ts). */
export const NOTICE_KEY = 'bible.read.movedNotice';
/** On the old address: the hash of the library simplymybible.com confirmed it has. */
export const SENT_KEY = 'bible.read.movedTo';

/** Characters of the encoded library per address: well under every browser's URL limit. */
export const CHUNK = 60_000;
/** At most this many pieces (about 3.8 MB encoded, more than localStorage holds). */
export const MAX_CHUNKS = 64;
/** Automatic backups kept (app.svelte.ts). */
export const BACKUPS_KEPT = 10;

// ---------------------------------------------------------------- reading and encoding

/**
 * What the old address holds, or null when it holds nothing.
 * @param {{ get(key: string): string | null }} storage
 */
export function collect(storage) {
	const get = (/** @type {string} */ k) => storage.get(k);
	const payload = {
		v: 1,
		settings: get(KEYS.settings),
		study: get(KEYS.study),
		backups: get(KEYS.backups),
		theme: get(KEYS.theme),
		/** Imported translations' files stay behind (they are often megabytes): their names come. */
		translations: importedNames(get(KEYS.imported))
	};
	return payload.settings || payload.study || payload.backups || payload.theme ? payload : null;
}

/** @param {string | null} raw */
function importedNames(raw) {
	try {
		const list = JSON.parse(raw ?? '[]');
		return Array.isArray(list)
			? list.filter((t) => t && typeof t.abbrev === 'string').map((t) => String(t.abbrev))
			: [];
	} catch {
		return [];
	}
}

/**
 * The hash that names a library: 24 hex digits of its SHA-256.
 * @param {string} text
 */
export async function digest(text) {
	const hash = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(text));
	return [...new Uint8Array(hash)]
		.slice(0, 12)
		.map((b) => b.toString(16).padStart(2, '0'))
		.join('');
}

/** @param {Uint8Array} bytes */
function toBase64Url(bytes) {
	let s = '';
	for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
	return btoa(s).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

/** @param {string} text */
function fromBase64Url(text) {
	const s = atob(text.replace(/-/g, '+').replace(/_/g, '/'));
	const bytes = new Uint8Array(s.length);
	for (let i = 0; i < s.length; i++) bytes[i] = s.charCodeAt(i);
	return bytes;
}

/**
 * @param {Uint8Array<ArrayBuffer>} bytes
 * @param {CompressionStream | DecompressionStream} stream
 */
async function pipe(bytes, stream) {
	const out = new Response(new Blob([bytes]).stream().pipeThrough(stream));
	return new Uint8Array(await out.arrayBuffer());
}

/**
 * Text as URL-safe characters: gzipped ("z…") where the browser can, as it is ("p…") where not.
 * @param {string} text
 * @param {boolean} [gzip]
 */
export async function encode(text, gzip = typeof CompressionStream === 'function') {
	const bytes = new TextEncoder().encode(text);
	return gzip ? `z${toBase64Url(await pipe(bytes, new CompressionStream('gzip')))}` : `p${toBase64Url(bytes)}`;
}

/** @param {string} encoded */
export async function decode(encoded) {
	const bytes = fromBase64Url(encoded.slice(1));
	if (encoded[0] === 'p') return new TextDecoder().decode(bytes);
	if (encoded[0] === 'z') return new TextDecoder().decode(await pipe(bytes, new DecompressionStream('gzip')));
	throw new Error('unknown encoding');
}

/**
 * @param {string} text
 * @param {number} [size]
 */
export function split(text, size = CHUNK) {
	const out = [];
	for (let i = 0; i < text.length; i += size) out.push(text.slice(i, i + size));
	return out.length ? out : [''];
}

/**
 * A path on this site to open afterwards, or null for anything else (another site, "//evil").
 * @param {string | null | undefined} path
 */
export function safePath(path) {
	if (typeof path !== 'string' || path.length > 2048 || !path.startsWith('/')) return null;
	if (path.startsWith('//') || path.startsWith('/\\') || /[\u0000-\u001f]/.test(path)) return null;
	return path;
}

/** @param {string} hash */
function fragment(hash) {
	return new URLSearchParams(hash.replace(/^#/, ''));
}

// ---------------------------------------------------------------- merging

/**
 * A study export (the core's JSON), or null when the text isn't one.
 * @param {string | null | undefined} raw
 */
export function parseStudy(raw) {
	if (!raw) return null;
	try {
		const doc = JSON.parse(raw);
		return doc && typeof doc === 'object' && doc.app === 'bible-rs' ? doc : null;
	} catch {
		return null;
	}
}

/** @param {any} doc */
export function studyCounts(doc) {
	const n = (/** @type {unknown} */ a) => (Array.isArray(a) ? a.length : 0);
	return {
		highlights: n(doc?.highlights),
		notes: n(doc?.notes),
		bookmarks: n(doc?.bookmarks),
		history: n(doc?.history),
		plans: n(doc?.plans)
	};
}

/** @param {any} doc */
export function hasStudy(doc) {
	return Object.values(studyCounts(doc)).some((n) => n > 0);
}

/** @param {unknown} s */
const time = (s) => (typeof s === 'string' ? Date.parse(s) || 0 : 0);
/** @param {any} e */
const verseKey = (e) => `${e.book}.${e.chapter}.${e.verse}`;
/** @param {unknown} a @returns {any[]} */
const list = (a) => (Array.isArray(a) ? a : []);

/**
 * Entries by key; `newer(a, b)` says whether incoming `b` replaces local `a`.
 * @param {any[]} local
 * @param {any[]} incoming
 * @param {(e: any) => string} key
 * @param {(a: any, b: any) => any} pick
 */
function byKey(local, incoming, key, pick) {
	/** @type {Map<string, any>} */
	const out = new Map();
	for (const e of local) out.set(key(e), e);
	for (const e of incoming) {
		const k = key(e);
		out.set(k, out.has(k) ? pick(out.get(k), e) : e);
	}
	return [...out.values()];
}

/**
 * Merges the study saved on the old address into the one here. Nothing is deleted. Where both
 * have the same verse, the one edited last wins (a tie keeps this address's): a verse's
 * highlights together, as the core imports them; a note by its `updated`. A bookmark here is
 * kept (bookmarks have no edit time). Reading sessions are joined (the same session saved
 * twice keeps the longer save), and plans too (the same plan's ticked passages are joined; a plan
 * started again keeps the later start).
 *
 * @param {string | null} localRaw the study here
 * @param {string | null} incomingRaw the old address's
 * @returns {string | null} what to save here
 */
export function mergeStudy(localRaw, incomingRaw) {
	const local = parseStudy(localRaw);
	const incoming = parseStudy(incomingRaw);
	if (!incoming) return localRaw;
	if (!local) return incomingRaw;

	/** @param {any[]} items */
	const groups = (items) => {
		/** @type {Map<string, any[]>} */
		const m = new Map();
		for (const h of items) m.set(verseKey(h), [...(m.get(verseKey(h)) ?? []), h]);
		return m;
	};
	/** @param {any[]} g */
	const edited = (g) => Math.max(...g.map((h) => time(h.edited ?? h.created)));
	const mine = groups(list(local.highlights));
	for (const [k, g] of groups(list(incoming.highlights))) {
		const have = mine.get(k);
		if (!have || edited(g) > edited(have)) mine.set(k, g);
	}

	const plans = byKey(list(local.plans), list(incoming.plans), (p) => String(p.id), (a, b) => {
		if (time(b.started) > time(a.started)) return b;
		if (time(b.started) < time(a.started)) return a;
		/** @type {Map<string, [number, number]>} */
		const read = new Map();
		for (const r of [...list(a.read), ...list(b.read)]) read.set(`${r[0]}.${r[1]}`, r);
		const p = { ...a, read: [...read.values()] };
		if (!p.finished && b.finished) p.finished = b.finished;
		return p;
	});

	const merged = {
		...local,
		format: Math.max(Number(local.format) || 1, Number(incoming.format) || 1),
		highlights: [...mine.values()].flat(),
		notes: byKey(list(local.notes), list(incoming.notes), verseKey, (a, b) =>
			time(b.updated) > time(a.updated) ? b : a
		),
		bookmarks: byKey(list(local.bookmarks), list(incoming.bookmarks), verseKey, (a) => a),
		history: byKey(list(local.history), list(incoming.history), (r) => String(r.started), (a, b) =>
			(Number(b.seconds) || 0) > (Number(a.seconds) || 0) ? b : a
		),
		plans
	};
	return JSON.stringify(merged);
}

/**
 * Both addresses' automatic backups, newest first, with `extra` (the study here before the
 * merge) kept among them.
 * @param {string | null} localRaw
 * @param {string | null} incomingRaw
 * @param {{ at: number; json: string } | null} extra
 */
export function mergeBackups(localRaw, incomingRaw, extra) {
	/** @param {string | null} raw @returns {{ at: number; json: string }[]} */
	const parse = (raw) => {
		try {
			const l = JSON.parse(raw ?? '[]');
			return Array.isArray(l) ? l.filter((b) => b && typeof b.at === 'number' && typeof b.json === 'string') : [];
		} catch {
			return [];
		}
	};
	const all = [...(extra ? [extra] : []), ...parse(localRaw), ...parse(incomingRaw)];
	const seen = new Set();
	const out = all
		.filter((b) => {
			const k = `${b.at}\n${b.json}`;
			if (seen.has(k)) return false;
			seen.add(k);
			return true;
		})
		.sort((a, b) => b.at - a.at);
	// The study from before the merge stays, however many backups there are.
	const kept = out.slice(0, BACKUPS_KEPT);
	if (extra && !kept.includes(extra)) kept[kept.length - 1] = extra;
	return kept.sort((a, b) => b.at - a.at);
}

/**
 * The settings to keep: this address's once it has a library of its own (the reader set it up
 * here), otherwise the old address's.
 * @param {string | null} localRaw
 * @param {string | null} incomingRaw
 * @param {boolean} localHasStudy
 */
export function pickSettings(localRaw, incomingRaw, localHasStudy) {
	const valid = (/** @type {string | null} */ raw) => {
		try {
			const s = JSON.parse(raw ?? 'null');
			return s !== null && typeof s === 'object' && !Array.isArray(s);
		} catch {
			return false;
		}
	};
	if (!valid(incomingRaw)) return localRaw;
	if (valid(localRaw) && localHasStudy) return localRaw;
	return incomingRaw;
}

/**
 * Merges a library from the old address into this address's storage.
 * @param {{ get(key: string): string | null; set(key: string, value: string): void }} storage
 * @param {any} payload what `collect` read on the old address
 * @param {number} now unix seconds
 */
export function applyMove(storage, payload, now) {
	if (!payload || payload.v !== 1) throw new Error('not a library from the old address');
	const str = (/** @type {unknown} */ v) => (typeof v === 'string' ? v : null);
	const localStudy = storage.get(KEYS.study);
	const local = parseStudy(localStudy);
	const study = mergeStudy(localStudy, str(payload.study));
	// The study here before the merge, as a backup, when the merge changes it.
	const before = local && hasStudy(local) && study !== localStudy ? { at: now, json: localStudy ?? '' } : null;
	const backups = mergeBackups(storage.get(KEYS.backups), str(payload.backups), before);
	const settings = pickSettings(storage.get(KEYS.settings), str(payload.settings), hasStudy(local));

	// The backups first, so the study from before the merge is kept before it changes.
	if (backups.length) {
		try {
			storage.set(KEYS.backups, JSON.stringify(backups));
		} catch {
			// Full storage: the study matters more than its backups (the old address keeps its own).
			if (before) storage.set(KEYS.backups, JSON.stringify([before]));
		}
	}
	if (study !== null && study !== localStudy) storage.set(KEYS.study, study);
	if (settings !== null && settings !== storage.get(KEYS.settings)) storage.set(KEYS.settings, settings);
	const theme = str(payload.theme);
	if (theme && storage.get(KEYS.theme) === null) storage.set(KEYS.theme, theme);

	const counts = studyCounts(parseStudy(str(payload.study)));
	return {
		highlights: counts.highlights,
		notes: counts.notes,
		bookmarks: counts.bookmarks,
		translations: Array.isArray(payload.translations) ? payload.translations.map(String) : []
	};
}

/** @param {{ get(key: string): string | null }} storage */
export function movedIds(storage) {
	try {
		const l = JSON.parse(storage.get(MOVED_KEY) ?? '[]');
		return Array.isArray(l) ? l.filter((m) => m && typeof m.id === 'string').map((m) => m.id) : [];
	} catch {
		return [];
	}
}

// ---------------------------------------------------------------- the two pages

/**
 * @typedef {{ get(key: string): string | null; set(key: string, value: string): void; remove(key: string): void; keys(): string[] }} Store
 * @typedef {{
 *   href: string;
 *   referrer: string;
 *   storage: Store;
 *   session: Store;
 *   go(url: string): void;
 *   show(state: string, info?: any): void;
 *   ask(): Promise<boolean>;
 *   now(): number;
 * }} Env
 */

/** @param {string} referrer @param {string} origin */
const from = (referrer, origin) => referrer === origin || referrer.startsWith(`${origin}/`);

/**
 * The old address: sends its library on, or the reader straight on when it has none.
 * @param {Env} env
 * @param {{ old?: string; new?: string }} [at] the addresses (tests run them on localhost)
 */
export async function leave(env, at = {}) {
	const oldOrigin = at.old ?? OLD;
	const newOrigin = at.new ?? NEW;
	const url = new URL(env.href);
	const f = fragment(url.hash);
	const ours = f.has('smb-next') || f.has('smb-done') || f.has('keep');
	const to = safePath(f.get('to')) ?? `${url.pathname}${url.search}${ours ? '' : url.hash}`;
	const onward = `${newOrigin}${to}`;
	// Only the old address has a library to send (simplymybible.com serves this file too).
	if (url.origin !== oldOrigin) return env.go(`${newOrigin}/`);

	if (f.has('keep')) return env.show('keep', { study: env.storage.get(KEYS.study) });
	// simplymybible.com has the library (a made-up hash here only ever names no library).
	const done = f.get('smb-done');
	if (done !== null) {
		if (/^[0-9a-f]{24}$/.test(done)) env.storage.set(SENT_KEY, done);
		return env.go(onward);
	}

	let payload;
	try {
		payload = collect(env.storage);
	} catch {
		payload = null; // storage blocked here: there's nothing to bring
	}
	if (!payload) return env.go(onward);
	let json = JSON.stringify(payload);
	let id = await digest(json);
	const next = f.get('smb-next')?.split('.');
	if (!next && env.storage.get(SENT_KEY) === id) return env.go(onward);

	let chunks = split(await encode(json));
	if (chunks.length > MAX_CHUNKS) {
		// Too big to carry: the backups stay behind (the study itself is what matters).
		json = JSON.stringify({ ...payload, backups: null });
		id = await digest(json);
		chunks = split(await encode(json));
		if (chunks.length > MAX_CHUNKS) return env.show('error', { keep: `${oldOrigin}/#keep` });
	}
	const i = next && next[0] === id && Number(next[1]) < chunks.length ? Math.max(0, Number(next[1]) | 0) : 0;
	env.go(
		`${newOrigin}/move/in.html#smb=${id}.${i}.${chunks.length}.${chunks[i]}&to=${encodeURIComponent(to)}`
	);
}

/**
 * simplymybible.com: takes a piece, asks for the next, and merges the library once it has them all.
 * @param {Env} env
 * @param {{ old?: string; new?: string }} [at]
 */
export async function arrive(env, at = {}) {
	const oldOrigin = at.old ?? OLD;
	const url = new URL(env.href);
	const f = fragment(url.hash);
	const to = safePath(f.get('to')) ?? '/read';
	const here = new URL(to, env.href).href;
	const back = (/** @type {string} */ what) => `${oldOrigin}/#${what}&to=${encodeURIComponent(to)}`;
	const m = /^([0-9a-f]{24})\.(\d+)\.(\d+)\.([A-Za-z0-9_-]*)$/.exec(f.get('smb') ?? '');
	if (!m || url.origin !== (at.new ?? NEW)) return env.go(here);
	const [, id, iText, nText, chunk] = m;
	const i = Number(iText);
	const n = Number(nText);
	if (n < 1 || n > MAX_CHUNKS || i >= n) return env.go(here);
	if (movedIds(env.storage).includes(id)) return env.go(back(`smb-done=${id}`));

	const key = (/** @type {number | string} */ k) => `smb.${id}.${k}`;
	const clear = () => {
		for (const k of env.session.keys()) if (k.startsWith(`smb.${id}.`)) env.session.remove(k);
	};
	const fail = (/** @type {string} */ why) => {
		clear();
		env.show('error', { why, keep: `${oldOrigin}/#keep`, to });
	};
	try {
		env.session.set(key(i), chunk);
		// A piece that didn't come from the old address means someone else made the link: ask.
		if (!from(env.referrer, oldOrigin)) env.session.set(key('ask'), '1');
		const hops = Number(env.session.get(key('hops')) ?? 0) + 1;
		env.session.set(key('hops'), String(hops));
		if (hops > n * 3) return fail('the pieces of your library kept going missing');
	} catch {
		return fail('this browser blocks the storage the move needs');
	}
	for (let k = 0; k < n; k++) if (env.session.get(key(k)) === null) return env.go(back(`smb-next=${id}.${k}`));

	let payload;
	try {
		const encoded = Array.from({ length: n }, (_, k) => env.session.get(key(k))).join('');
		const json = await decode(encoded);
		if ((await digest(json)) !== id) return fail('your library arrived damaged');
		payload = JSON.parse(json);
	} catch {
		return fail('your library arrived damaged');
	}
	if (env.session.get(key('ask')) && !(await env.ask())) {
		clear();
		return env.go(here);
	}
	try {
		const summary = applyMove(env.storage, payload, env.now());
		const moved = JSON.parse(env.storage.get(MOVED_KEY) ?? '[]');
		env.storage.set(
			MOVED_KEY,
			JSON.stringify([...(Array.isArray(moved) ? moved : []), { id, at: env.now(), from: new URL(oldOrigin).host }])
		);
		env.storage.set(NOTICE_KEY, JSON.stringify({ from: new URL(oldOrigin).host, ...summary }));
	} catch (e) {
		return fail(`this browser couldn't save it (${e instanceof Error ? e.message : e})`);
	}
	clear();
	env.go(back(`smb-done=${id}`));
}

// ---------------------------------------------------------------- in a browser

/** @param {Storage} s @returns {Store} */
function store(s) {
	return {
		get: (k) => s.getItem(k),
		set: (k, v) => s.setItem(k, v),
		remove: (k) => s.removeItem(k),
		keys: () => Array.from({ length: s.length }, (_, i) => s.key(i) ?? '')
	};
}

/**
 * The page's own environment. The address leaves the bar before anything else runs, so the
 * library in it isn't kept in history or shown.
 * @returns {Env}
 */
export function browser() {
	const href = location.href;
	if (location.hash) history.replaceState(null, '', location.pathname + location.search);
	return {
		href,
		referrer: document.referrer,
		storage: store(localStorage),
		session: store(sessionStorage),
		go: (url) => location.replace(url),
		show(state, info = {}) {
			document.body.dataset.state = state;
			const continueLink = document.querySelector('[data-continue]');
			if (continueLink && info.to) continueLink.setAttribute('href', info.to);
			const keepLink = document.querySelector('[data-keep]');
			if (keepLink && info.keep) keepLink.setAttribute('href', info.keep);
			const why = document.querySelector('[data-why]');
			if (why && info.why) why.textContent = info.why;
			const download = document.querySelector('[data-download]');
			if (download) {
				download.toggleAttribute('hidden', !info.study);
				download.addEventListener('click', () => {
					const a = document.createElement('a');
					a.href = URL.createObjectURL(new Blob([info.study], { type: 'application/json' }));
					a.download = 'bible-notes.json';
					a.click();
				});
			}
		},
		ask() {
			document.body.dataset.state = 'ask';
			return new Promise((resolve) => {
				document.querySelector('[data-yes]')?.addEventListener('click', () => resolve(true), { once: true });
				document.querySelector('[data-no]')?.addEventListener('click', () => resolve(false), { once: true });
			});
		},
		now: () => Math.floor(Date.now() / 1000)
	};
}
