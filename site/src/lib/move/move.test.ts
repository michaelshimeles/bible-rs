import { randomBytes } from 'node:crypto';
import { mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { beforeAll, describe, expect, it } from 'vitest';
import * as wasm from '../read/wasm/bible.js';
import { NOTICE_KEY as READER_NOTICE_KEY, takeMovedNotice } from '../read/moved';
import { SETTINGS_KEY, STUDY_KEY } from '../read/settings';
import { IMPORTED_KEY } from '../read/imports';
import { serveOldAddress } from '../../../scripts/vercel-routes.js';
import {
	BACKUPS_KEPT,
	CHUNK,
	KEYS,
	MOVED_KEY,
	NEW,
	NOTICE_KEY,
	OLD,
	SENT_KEY,
	arrive,
	decode,
	digest,
	encode,
	leave,
	mergeBackups,
	mergeStudy,
	pickSettings,
	safePath,
	split
} from '../../../static/move/move.js';

// ---------------------------------------------------------------- fixtures

const verse = (book: string, chapter: number, v: number) => ({
	reference: `${book} ${chapter}:${v}`,
	book,
	chapter,
	verse: v
});
const at = (day: number) => `2026-10-${String(day).padStart(2, '0')}T12:00:00Z`;
const note = (book: string, c: number, v: number, body: string, updated: number) => ({
	...verse(book, c, v),
	note: body,
	created: at(1),
	updated: at(updated)
});
const highlight = (book: string, c: number, v: number, color: string, created: number) => ({
	...verse(book, c, v),
	color,
	created: at(created)
});
const bookmark = (book: string, c: number, v: number, label: string) => ({ ...verse(book, c, v), label, created: at(2) });
const reading = (day: number, seconds: number) => ({
	started: at(day),
	book: 'JHN',
	chapter: 3,
	translation: 'kjv',
	seconds,
	furthest: 10,
	verses: 36
});
const study = (parts: Record<string, unknown[]>) =>
	JSON.stringify({ app: 'bible-rs', format: 1, exported_at: at(20), translation: '', ...parts });
const parsed = (s: string | null) => JSON.parse(s ?? 'null');

/** A Storage-like map, as the pages see localStorage and sessionStorage. */
class Store {
	map = new Map<string, string>();
	constructor(init: Record<string, string> = {}) {
		for (const [k, v] of Object.entries(init)) this.map.set(k, v);
	}
	get = (k: string) => this.map.get(k) ?? null;
	set = (k: string, v: string) => void this.map.set(k, v);
	remove = (k: string) => void this.map.delete(k);
	keys = () => [...this.map.keys()];
}

/**
 * Follows the pages from `start` the way a browser would, each address with its own storage, until
 * one opens a page that isn't a move page; returns that address and the addresses on the way.
 */
async function browse(
	start: string,
	old: Store,
	now: Store,
	opts: { ask?: boolean; referrer?: boolean; tamper?: (url: string) => string } = {}
) {
	const session = new Store();
	const shown: string[] = [];
	const hops: string[] = [];
	let href = start;
	let referrer = '';
	for (let step = 0; step < 500; step++) {
		const url = new URL(href);
		const onOld = url.origin === OLD;
		if (!onOld && url.pathname !== '/move/in.html') return { href, hops, shown };
		let next: string | null = null;
		const env = {
			href,
			referrer: opts.referrer === false ? '' : referrer,
			storage: onOld ? old : now,
			session: onOld ? new Store() : session,
			go: (u: string) => void (next = u),
			show: (state: string) => void shown.push(state),
			ask: async () => (shown.push('ask'), opts.ask ?? true),
			now: () => 1_793_600_000
		};
		await (onOld ? leave(env) : arrive(env));
		if (next === null) return { href, hops, shown };
		hops.push(next);
		referrer = `${url.origin}/`;
		href = opts.tamper ? opts.tamper(next) : next;
	}
	throw new Error('the pages never stopped');
}

beforeAll(() => {
	wasm.initSync({ module: readFileSync('src/lib/read/wasm/bible_bg.wasm') });
});

// ---------------------------------------------------------------- merging

describe('mergeStudy', () => {
	const local = study({
		highlights: [highlight('JHN', 3, 16, 'yellow', 10), highlight('GEN', 1, 1, 'green', 3)],
		notes: [note('JHN', 3, 16, 'here, newer', 12), note('ROM', 8, 28, 'here, older', 4)],
		bookmarks: [bookmark('PSA', 23, 1, 'here')],
		history: [reading(5, 60)],
		plans: [{ id: 'gospels', started: at(1), read: [[1, 0]] }]
	});
	const incoming = study({
		highlights: [highlight('JHN', 3, 16, 'pink', 8), highlight('GEN', 1, 1, 'blue', 9), highlight('MAT', 5, 3, 'orange', 2)],
		notes: [note('JHN', 3, 16, 'old address, older', 11), note('ROM', 8, 28, 'old address, newer', 9), note('PHP', 4, 13, 'only there', 2)],
		bookmarks: [bookmark('PSA', 23, 1, 'there'), bookmark('ISA', 40, 31, 'there')],
		history: [reading(5, 90), reading(6, 30)],
		plans: [
			{ id: 'gospels', started: at(1), finished: at(30), read: [[1, 0], [2, 0]] },
			{ id: 'psalms', started: at(3), read: [[1, 0]] }
		]
	});

	it('takes either side whole when the other has nothing', () => {
		expect(mergeStudy(null, incoming)).toBe(incoming);
		expect(mergeStudy(local, null)).toBe(local);
		expect(mergeStudy(local, 'not json')).toBe(local);
		expect(mergeStudy('{"app":"other"}', incoming)).toBe(incoming);
	});

	it('keeps the newer edit of each verse and deletes nothing', () => {
		const m = parsed(mergeStudy(local, incoming));
		const notes = Object.fromEntries(m.notes.map((n: any) => [n.reference, n.note]));
		expect(notes).toEqual({ 'JHN 3:16': 'here, newer', 'ROM 8:28': 'old address, newer', 'PHP 4:13': 'only there' });
		const colors = Object.fromEntries(m.highlights.map((h: any) => [h.reference, h.color]));
		expect(colors).toEqual({ 'JHN 3:16': 'yellow', 'GEN 1:1': 'blue', 'MAT 5:3': 'orange' });
		const marks = Object.fromEntries(m.bookmarks.map((b: any) => [b.reference, b.label]));
		expect(marks).toEqual({ 'PSA 23:1': 'here', 'ISA 40:31': 'there' });
		expect(m.history.map((r: any) => [r.started, r.seconds])).toEqual([
			[at(5), 90],
			[at(6), 30]
		]);
		expect(m.plans).toEqual([
			{ id: 'gospels', started: at(1), finished: at(30), read: [[1, 0], [2, 0]] },
			{ id: 'psalms', started: at(3), read: [[1, 0]] }
		]);
	});

	it('keeps this address on a tie', () => {
		const a = study({ notes: [note('JHN', 1, 1, 'here', 5)] });
		const b = study({ notes: [note('JHN', 1, 1, 'there', 5)] });
		expect(parsed(mergeStudy(a, b)).notes[0].note).toBe('here');
	});

	it('takes a plan started again on either side whole', () => {
		const a = study({ plans: [{ id: 'p', started: at(1), read: [[1, 0], [2, 0]] }] });
		const b = study({ plans: [{ id: 'p', started: at(9), read: [[1, 0]] }] });
		expect(parsed(mergeStudy(a, b)).plans).toEqual([{ id: 'p', started: at(9), read: [[1, 0]] }]);
		expect(parsed(mergeStudy(b, a)).plans).toEqual([{ id: 'p', started: at(9), read: [[1, 0]] }]);
	});

	it('gives the same result merged twice', () => {
		const once = mergeStudy(local, incoming);
		expect(parsed(mergeStudy(once, incoming))).toEqual(parsed(once));
	});

	it('gives the core a study it restores', () => {
		const core = new wasm.Bible();
		core.restore(mergeStudy(local, incoming)!);
		expect(core.counts()).toMatchObject({ highlights: 3, notes: 3, bookmarks: 2 });
		const again = parsed(core.snapshot());
		expect(again.plans).toHaveLength(2);
		expect(again.history).toHaveLength(2);
		// The core's own snapshots merge too (the reader saves those).
		const other = new wasm.Bible();
		other.restore(incoming);
		expect(parsed(mergeStudy(core.snapshot(), other.snapshot())).notes).toHaveLength(3);
	});
});

describe('pickSettings', () => {
	const here = '{"theme":"dark"}';
	const there = '{"theme":"sepia"}';
	it("keeps this address's once it has a library, else the old address's", () => {
		expect(pickSettings(here, there, true)).toBe(here);
		expect(pickSettings(here, there, false)).toBe(there);
		expect(pickSettings(null, there, true)).toBe(there);
		expect(pickSettings(here, null, false)).toBe(here);
		expect(pickSettings(here, '[1]', false)).toBe(here);
	});
});

describe('mergeBackups', () => {
	const b = (at: number) => ({ at, json: `{"n":${at}}` });
	it('joins both, newest first, without repeats', () => {
		const out = mergeBackups(JSON.stringify([b(5), b(3)]), JSON.stringify([b(4), b(3)]), null);
		expect(out.map((x) => x.at)).toEqual([5, 4, 3]);
	});
	it('keeps the study from before the merge when the list is full', () => {
		const many = JSON.stringify(Array.from({ length: 12 }, (_, i) => b(100 + i)));
		const before = { at: 50, json: 'before' };
		const out = mergeBackups(many, many, before);
		expect(out).toHaveLength(BACKUPS_KEPT);
		expect(out).toContainEqual(before);
	});
});

// ---------------------------------------------------------------- carrying

describe('encoding', () => {
	it('round-trips text, gzipped or not', async () => {
		const text = `{"note":"In the beginning — ✝ 🙏 ${'x'.repeat(1000)}"}`;
		for (const gzip of [true, false]) {
			const e = await encode(text, gzip);
			expect(e).toMatch(/^[zp][A-Za-z0-9_-]*$/);
			expect(await decode(e)).toBe(text);
		}
		expect((await encode(text, true)).length).toBeLessThan((await encode(text, false)).length);
	});
	it('splits into pieces', () => {
		expect(split('abcde', 2)).toEqual(['ab', 'cd', 'e']);
		expect(split('', 2)).toEqual(['']);
	});
	it('names a library by its hash', async () => {
		expect(await digest('a')).toBe('ca978112ca1bbdcafac231b3');
	});
	it('only opens paths on this site afterwards', () => {
		expect(safePath('/read/kjv/john/3?x=1#v16')).toBe('/read/kjv/john/3?x=1#v16');
		for (const p of ['//evil.example', '/\\evil.example', 'https://evil.example', 'read', '/a\nb', null]) {
			expect(safePath(p)).toBeNull();
		}
	});
});

describe('moving a library', () => {
	const oldLibrary = () =>
		new Store({
			[STUDY_KEY]: study({ notes: [note('JHN', 3, 16, 'from the old address', 9)], bookmarks: [bookmark('PSA', 23, 1, 'x')] }),
			[SETTINGS_KEY]: '{"theme":"sepia","onboarded":true}',
			[KEYS.backups]: JSON.stringify([{ at: 1, json: study({}) }]),
			[IMPORTED_KEY]: JSON.stringify([{ id: 'imported-abc', abbrev: 'ABC', imported: true }]),
			theme: 'light'
		});

	it('uses the keys the reader saves', () => {
		expect(KEYS).toMatchObject({ settings: SETTINGS_KEY, study: STUDY_KEY, imported: IMPORTED_KEY });
		expect(NOTICE_KEY).toBe(READER_NOTICE_KEY);
	});

	it('sends a reader with nothing saved straight on', async () => {
		const r = await browse(`${OLD}/read/kjv/john/3?x=1#v16`, new Store(), new Store());
		expect(r.hops).toEqual([`${NEW}/read/kjv/john/3?x=1#v16`]);
	});

	it('brings the library over and opens the page asked for', async () => {
		const old = oldLibrary();
		const here = new Store();
		const r = await browse(`${OLD}/read/kjv/john/3#v16`, old, here);
		expect(r.href).toBe(`${NEW}/read/kjv/john/3#v16`);
		expect(r.hops.map((h) => new URL(h).origin + new URL(h).pathname)).toEqual([
			`${NEW}/move/in.html`,
			`${OLD}/`,
			`${NEW}/read/kjv/john/3`
		]);
		expect(here.get(STUDY_KEY)).toBe(old.get(STUDY_KEY));
		expect(here.get(SETTINGS_KEY)).toBe(old.get(SETTINGS_KEY));
		expect(here.get('theme')).toBe('light');
		expect(here.get(IMPORTED_KEY)).toBeNull();
		expect(parsed(here.get(NOTICE_KEY))).toEqual({
			from: 'bible.rasmic.xyz',
			highlights: 0,
			notes: 1,
			bookmarks: 1,
			translations: ['ABC']
		});
		expect(old.get(SENT_KEY)).toBe(parsed(here.get(MOVED_KEY))[0].id);
		// The old address keeps its copy.
		expect(old.get(STUDY_KEY)).toBe(oldLibrary().get(STUDY_KEY));
		// Next time it goes straight on.
		expect((await browse(`${OLD}/`, old, here)).hops).toEqual([`${NEW}/`]);
	});

	it('never merges the same library twice, so what was deleted here stays deleted', async () => {
		const old = oldLibrary();
		const here = new Store();
		await browse(`${OLD}/read`, old, here);
		here.set(STUDY_KEY, study({ notes: [] }));
		old.remove(SENT_KEY); // as if the old address never heard back
		const r = await browse(`${OLD}/read`, old, here);
		expect(r.href).toBe(`${NEW}/read`);
		expect(parsed(here.get(STUDY_KEY)).notes).toEqual([]);
		expect(old.get(SENT_KEY)).not.toBeNull();
	});

	it('merges into a library already here, keeping a backup of it', async () => {
		const old = oldLibrary();
		const mine = study({ notes: [note('ROM', 8, 28, 'here', 15)] });
		const here = new Store({ [STUDY_KEY]: mine, [SETTINGS_KEY]: '{"theme":"dark"}' });
		await browse(`${OLD}/read`, old, here);
		expect(parsed(here.get(STUDY_KEY)).notes.map((n: any) => n.note).sort()).toEqual(['from the old address', 'here']);
		expect(here.get(SETTINGS_KEY)).toBe('{"theme":"dark"}');
		expect(parsed(here.get(KEYS.backups))).toContainEqual({ at: 1_793_600_000, json: mine });
	});

	it('carries a big library in pieces', async () => {
		// Random text, so it doesn't compress into one piece.
		const notes = Array.from({ length: 3000 }, (_, i) =>
			note('PSA', 119, 1 + (i % 176), randomBytes(48).toString('base64'), 9)
		);
		const old = new Store({ [STUDY_KEY]: study({ notes }) });
		const here = new Store();
		const r = await browse(`${OLD}/read`, old, here);
		const pieces = r.hops.filter((h) => h.startsWith(`${NEW}/move/in.html`));
		expect(pieces.length).toBeGreaterThan(2);
		expect(pieces.every((h) => h.length < CHUNK + 200)).toBe(true);
		expect(r.href).toBe(`${NEW}/read`);
		expect(here.get(STUDY_KEY)).toBe(old.get(STUDY_KEY));
	});

	it("asks first when the link didn't come from the old address", async () => {
		const here = new Store();
		const declined = await browse(`${OLD}/read`, oldLibrary(), here, { referrer: false, ask: false });
		expect(declined.shown).toContain('ask');
		expect(declined.href).toBe(`${NEW}/read`);
		expect(here.get(STUDY_KEY)).toBeNull();
		const accepted = await browse(`${OLD}/read`, oldLibrary(), here, { referrer: false, ask: true });
		expect(accepted.shown).toContain('ask');
		expect(here.get(STUDY_KEY)).not.toBeNull();
	});

	it('changes nothing when a piece arrives damaged', async () => {
		const here = new Store({ [STUDY_KEY]: study({ notes: [note('ROM', 8, 28, 'here', 15)] }) });
		const before = [...here.map];
		const tamper = (u: string) => (u.includes('/move/in.html') ? u.replace(/\.z(.)/, (_, c) => `.z${c === 'A' ? 'B' : 'A'}`) : u);
		const r = await browse(`${OLD}/read`, oldLibrary(), here, { tamper });
		expect(r.shown).toEqual(['error']);
		expect([...here.map]).toEqual(before);
	});

	it('runs each page only at its own address', async () => {
		let gone = '';
		const here = new Store({ [STUDY_KEY]: study({ notes: [note('ROM', 8, 28, 'here', 15)] }) });
		await leave({
			href: `${NEW}/move/out.html`,
			referrer: '',
			storage: here,
			session: new Store(),
			go: (u: string) => void (gone = u),
			show: () => {},
			ask: async () => true,
			now: () => 0
		});
		expect(gone).toBe(`${NEW}/`);
		const env = {
			href: `${OLD}/move/in.html#smb=${'a'.repeat(24)}.0.1.zAAAA&to=/read`,
			referrer: '',
			storage: new Store(),
			session: new Store(),
			go: (u: string) => void (gone = u),
			show: () => {},
			ask: async () => true,
			now: () => 0
		};
		await arrive(env);
		expect(gone).toBe(`${OLD}/read`);
		expect(env.session.keys()).toEqual([]);
	});

	it('ignores links to other sites', async () => {
		const r = await browse(`${OLD}/read#to=//evil.example`, new Store(), new Store());
		expect(r.hops).toEqual([`${NEW}/read#to=//evil.example`]);
		const env = {
			href: `${NEW}/move/in.html#smb=nonsense&to=https://evil.example`,
			referrer: '',
			storage: new Store(),
			session: new Store(),
			go: (u: string) => void (gone = u),
			show: () => {},
			ask: async () => true,
			now: () => 0
		};
		let gone = '';
		await arrive(env);
		expect(gone).toBe(`${NEW}/read`);
	});
});

describe('the reader', () => {
	it('says once what came over', () => {
		const s = new Map<string, string>();
		const storage = {
			getItem: (k: string) => s.get(k) ?? null,
			removeItem: (k: string) => void s.delete(k)
		} as Storage;
		s.set(READER_NOTICE_KEY, JSON.stringify({ from: 'bible.rasmic.xyz', highlights: 1, notes: 2, bookmarks: 0, translations: ['ABC'] }));
		expect(takeMovedNotice(storage)).toEqual([
			'Brought 1 highlight, 2 notes, 0 bookmarks over from bible.rasmic.xyz',
			'Import ABC again: imported translations stay on bible.rasmic.xyz'
		]);
		expect(takeMovedNotice(storage)).toEqual([]);
	});
});

describe('serveOldAddress', () => {
	it("puts the old address's routes first", () => {
		const file = join(mkdtempSync(join(tmpdir(), 'routes-')), 'config.json');
		writeFileSync(file, JSON.stringify({ version: 3, routes: [{ handle: 'filesystem' }] }));
		serveOldAddress(file, 'bible.rasmic.xyz');
		const routes = JSON.parse(readFileSync(file, 'utf8')).routes;
		expect(routes.at(-1)).toEqual({ handle: 'filesystem' });
		const [sw, page] = routes;
		for (const r of [sw, page]) expect(r.has).toEqual([{ type: 'host', value: 'bible.rasmic.xyz' }]);
		expect(sw.dest).toBe('/move/sw.js');
		expect(page.dest).toBe('/move/out.html');
		const matches = (src: string, path: string) => new RegExp(src).test(path);
		expect(matches(sw.src, '/service-worker.js')).toBe(true);
		for (const p of ['/', '/read', '/read/kjv/john/3', '/john/3/16', '/service-worker.js']) expect(matches(page.src, p)).toBe(true);
		for (const p of ['/move/move.js', '/move/out.html', '/move/style.css']) expect(matches(page.src, p)).toBe(false);
	});
});
