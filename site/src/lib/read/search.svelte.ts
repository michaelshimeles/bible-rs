/**
 * Search: the query in the toolbar's field, the filters and results in the panel that slides in
 * from the right under it, and the navigator under the toolbar stepping through them (the search
 * half of crates/app/src/sidebar.rs). The core searches the whole translation in a few
 * milliseconds; hits are fetched in pages as the result list scrolls.
 *
 * As the query changes, the reader goes straight to a match in the book being read (the first at
 * or after where the search started, or the last match gone to); ↑ ↓ then step through every
 * match, into other books.
 */
import { untrack } from 'svelte';
import type { ReadApp } from './app.svelte';
import type { SearchHit, SearchPage, SearchScope } from './core';

/** Hits fetched with the count, then in pages of this many. */
export const FIRST = 2000;
const PAGE = 500;
/** How far into the book's matches the live search looks for one after the anchor. */
const LIVE_SCAN = 1000;

export class Search {
	query = $state('');
	scope = $state<SearchScope>('all');
	/** Hits in the book being read first, then the rest in canonical order. */
	currentFirst = $state(false);
	/** Whether the toolbar's field has focus (arrow keys then move through the results). */
	fieldFocused = $state(false);
	/** Bumped to ask the toolbar's field for focus. */
	focusRequest = $state(0);
	/** Bumped to scroll the selected result into view. */
	scrollRequest = $state(0);
	/** How long the last search took, in ms (read with `results`). */
	elapsed = 0;
	/** The hit gone to in the current results; -1 before one is (no match in the book read). */
	#selected = $state({ key: '', index: -1 });
	/** Where the live search looks from: the place read when the search began, then each hit gone to. */
	#anchor: { chapter: number; verse: number } | null = null;
	/** Whether this search has added its one entry to the back history yet. */
	#pushed = false;
	#generation = $state(0);
	#pages = new Map<number, SearchHit[]>();
	#pagesKey = '';
	private readonly app: ReadApp;

	constructor(app: ReadApp) {
		this.app = app;
	}

	get searching(): boolean {
		return this.query.trim() !== '';
	}

	/**
	 * The book "This book first" puts first: the one being read when the search changed, so going
	 * to a hit in another book doesn't reorder the results under it.
	 */
	#preferred = { search: '', book: -1 };

	/** What the results depend on; a change runs the search again and selects the first hit. */
	readonly key = $derived.by(() => {
		const s = this.app.settings;
		const search = JSON.stringify([s.translation, this.query.trim(), this.scope, s.wholeWords, this.currentFirst, this.#generation]);
		if (this.#preferred.search !== search) {
			this.#preferred = { search, book: this.currentFirst ? untrack(() => this.app.hereRef().book) : -1 };
		}
		return `${search}${this.#preferred.book}`;
	});

	/** The count, the books and the first hits, or null with no search (or no text yet). */
	readonly results = $derived.by((): SearchPage | null => {
		const app = this.app;
		const id = app.settings.translation;
		const key = this.key;
		if (!this.searching || !app.core || !app.loaded.has(id)) return null;
		const started = performance.now();
		const page = app.core.search(id, this.query, this.scope, app.settings.wholeWords, this.#first(), 0, FIRST);
		this.elapsed = performance.now() - started;
		this.#pages = new Map([[0, page.hits]]);
		this.#pagesKey = key;
		return page;
	});

	/** The selected hit, or -1 when none has been gone to yet. */
	get index(): number {
		return this.#selected.key === this.key ? this.#selected.index : -1;
	}

	/** For the navigator: `index` is 0 until a hit has been gone to. */
	get position() {
		const total = this.searching ? (this.results?.total ?? 0) : 0;
		return total ? { index: this.index + 1, total } : null;
	}

	#first(): number | null {
		return this.currentFirst ? this.#preferred.book : null;
	}

	/** The `i`th hit, fetching its page when it isn't in yet. */
	hit(i: number): SearchHit | null {
		const app = this.app;
		const page = this.results;
		if (!page || i < 0 || i >= page.total || !app.core) return null;
		if (this.#pagesKey !== this.key) return null;
		if (i < FIRST) return this.#pages.get(0)?.[i] ?? null;
		const n = Math.floor((i - FIRST) / PAGE);
		let hits = this.#pages.get(n + 1);
		if (!hits) {
			const s = app.settings;
			hits = app.core.search(s.translation, this.query, this.scope, s.wholeWords, this.#first(), FIRST + n * PAGE, PAGE).hits;
			this.#pages.set(n + 1, hits);
		}
		return hits[i - FIRST - n * PAGE] ?? null;
	}

	/** Selects a hit and goes to it (and selects its verse). One search adds one back entry. */
	choose(i: number) {
		const hit = this.hit(i);
		if (!hit) return;
		this.#selected = { key: this.key, index: i };
		this.#anchor = { chapter: hit.chapterIndex, verse: hit.verse.verse };
		const history = !this.#pushed;
		this.#pushed = true;
		this.app.dispatch({ type: 'jump', chapter: hit.chapterIndex, verse: hit.verse.verse, flash: true, history });
	}

	/**
	 * Called whenever the results change: goes to the anchor's book's first match at or after the
	 * anchor (or its first match), so typing finds the words in the book being read. With no match
	 * in that book nothing moves, and ↓ starts at the first result.
	 */
	live() {
		if (!this.searching) {
			this.#anchor = null;
			this.#pushed = false;
			return;
		}
		const page = this.results;
		if (!page) return;
		const app = this.app;
		const anchor = (this.#anchor ??= { chapter: app.here.chapter, verse: app.here.verse });
		const book = app.refAt(anchor.chapter, 1).book;
		// The book's hits are a run in the results; `books` counts the runs in order.
		let start = 0;
		let count = 0;
		for (const b of page.books) {
			if (b.book === book) {
				count = b.count;
				break;
			}
			start += b.count;
		}
		if (!count) return;
		let pick = start;
		const end = start + Math.min(count, LIVE_SCAN);
		for (let i = start; i < end; i++) {
			const h = this.hit(i);
			if (!h) break;
			if (h.chapterIndex > anchor.chapter || (h.chapterIndex === anchor.chapter && h.verse.verse >= anchor.verse)) {
				pick = i;
				break;
			}
		}
		this.choose(pick);
		this.scrollRequest++;
	}

	focus(query?: string) {
		if (query !== undefined) this.query = query;
		this.focusRequest++;
	}

	/** ↓ / ↑: the next or previous result, wherever it is, round from the last to the first. */
	step(delta: number) {
		const total = this.results?.total ?? 0;
		if (!total) return;
		const from = this.index < 0 && delta > 0 ? -1 : this.index < 0 ? total : this.index;
		this.choose((((from + delta) % total) + total) % total);
		this.scrollRequest++;
	}

	/** Return in the field: the selected hit, or the first. */
	open() {
		if (this.results?.total) this.choose(Math.max(0, this.index));
	}

	clear() {
		this.query = '';
	}

	invalidate() {
		this.#generation++;
	}
}

/** A hit's snippet: where it starts (a few words before the first match) and its runs. */
export interface Snippet {
	/** Whether it starts mid-verse (shown after "…"). */
	cut: boolean;
	runs: { text: string; hit: boolean }[];
}

/** Matches separated only by spaces read as one ("love one another"). */
function joined(text: string, matches: [number, number][]): [number, number][] {
	const out: [number, number][] = [];
	for (const [s, e] of matches) {
		const last = out[out.length - 1];
		if (last && /^ *$/.test(text.slice(last[1], s))) last[1] = e;
		else out.push([s, e]);
	}
	return out;
}

/** Five words before the first match, or the verse's start when the match is in its first seven. */
function kwicStart(text: string, first: number | undefined): number {
	if (first === undefined) return 0;
	const starts: number[] = [];
	for (let i = 0; i < first; i++) {
		if (!/\s/.test(text[i]) && (i === 0 || /\s/.test(text[i - 1]))) starts.push(i);
	}
	return starts.length <= 7 ? 0 : starts[starts.length - 5];
}

export function snippet(hit: SearchHit): Snippet {
	const { text } = hit;
	const ranges = joined(text, hit.matches);
	const start = kwicStart(text, ranges[0]?.[0]);
	const runs: Snippet['runs'] = [];
	let pos = start;
	for (const [s, e] of ranges) {
		if (s < start) continue;
		if (s > pos) runs.push({ text: text.slice(pos, s), hit: false });
		runs.push({ text: text.slice(s, e), hit: true });
		pos = e;
	}
	if (pos < text.length) runs.push({ text: text.slice(pos), hit: false });
	return { cut: start > 0, runs };
}

/** "1,234". */
export const group = (n: number) => n.toLocaleString('en-US');
