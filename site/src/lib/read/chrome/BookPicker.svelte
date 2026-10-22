<!--
	The book picker under the toolbar's location button (crates/app/src/book_picker.rs): the Old and
	New Testament books side by side, then a book's chapters, ten to a row. Arrow keys move through
	the books spatially across both testaments and through the chapters in order; return chooses,
	backspace goes back to the books.

	Typing finds a book: the letters typed (read as the jump palette reads names: "jn", "1co",
	"song") light the best match and fade the books they rule out, the arrows step through the
	matches best first, and return opens the lit one, or goes straight to a chapter typed after the
	name ("rom 8"). Backspace takes back a letter and escape clears them. On the chapters, typing a
	number lights that chapter.
-->
<script lang="ts">
	import { tick } from 'svelte';
	import { getApp } from '../app.svelte';
	import { filterBooks, parseReference, type Book } from '../core';
	import { Icon, Popover } from '../ui';

	interface Props {
		anchor: HTMLElement;
		onclose: () => void;
	}

	let { anchor, onclose }: Props = $props();

	const app = getApp();
	const COLUMNS = 3;
	const CHAPTER_COLUMNS = 10;
	/** Longer names show the book's abbreviation. */
	const MAX_NAME = 12;

	const books = $derived(app.core?.books ?? []);
	const firstNt = $derived(books.findIndex((b) => b.newTestament));
	const hereBook = $derived(books.findLast((b) => b.firstChapter <= app.here.chapter));

	let book = $state<Book | null>(null);
	let panel = $state<HTMLElement>();
	/** What's been typed: a book's name on the books, a chapter's number on the chapters. */
	let query = $state('');
	/** The books it could mean, best first. */
	const matches = $derived(query ? filterBooks(query) : []);

	const name = (b: Book) => (b.name.length > MAX_NAME ? b.abbrev : b.name);

	/** A book's column across both testaments (0–5) and its row. */
	function spot(i: number): [number, number] {
		const [group, k] = i < firstNt ? [0, i] : [1, i - firstNt];
		return [group * COLUMNS + (k % COLUMNS), Math.floor(k / COLUMNS)];
	}

	function at(col: number, row: number): number | null {
		if (col < 0 || row < 0 || col >= 2 * COLUMNS) return null;
		const [start, end] = col < COLUMNS ? [0, firstNt] : [firstNt, books.length];
		const i = start + row * COLUMNS + (col % COLUMNS);
		return i < end ? i : null;
	}

	const cells = () => [...(panel?.querySelectorAll<HTMLElement>('[data-cell]') ?? [])];
	const focused = () => {
		const el = document.activeElement as HTMLElement | null;
		return el?.dataset.cell !== undefined && panel?.contains(el) ? el : null;
	};

	function focusCell(key: string) {
		panel?.querySelector<HTMLElement>(`[data-cell="${key}"]`)?.focus({ preventScroll: true });
	}

	function chooseBook(b: Book, byKey: boolean) {
		if (b.chapters === 1) {
			app.dispatch({ type: 'jump', chapter: b.firstChapter });
			onclose();
			return;
		}
		book = b;
		query = '';
		// From the keyboard, the highlight lands on the chapter being read or the first.
		if (byKey) {
			const ch = hereBook?.index === b.index ? app.here.chapter - b.firstChapter : 0;
			void tick().then(() => focusCell(String(ch)));
		} else {
			void tick().then(() => panel?.focus({ preventScroll: true }));
		}
	}

	function chooseChapter(i: number) {
		if (!book) return;
		app.dispatch({ type: 'jump', chapter: book.firstChapter + i });
		onclose();
	}

	function backToBooks(byKey: boolean) {
		const from = book;
		book = null;
		query = '';
		void tick().then(() => (byKey && from ? focusCell(String(from.index)) : panel?.focus({ preventScroll: true })));
	}

	/** Typing on the books: light the best match (or nothing, when nothing matches). */
	async function retype(next: string) {
		query = next.trim() ? next : '';
		await tick();
		const best = matches[0];
		if (best !== undefined) focusCell(String(best));
		else panel?.focus({ preventScroll: true });
	}

	/** Typing on the chapters: light the chapter typed; a digit that runs past the last restarts. */
	function typeChapter(d: string) {
		if (!book) return;
		const n = book.chapters;
		const fits = (q: string) => Number(q) >= 1 && Number(q) <= n;
		query = fits(query + d) ? query + d : d;
		if (fits(query)) focusCell(String(Number(query) - 1));
	}

	/** Takes the keys typed into the picker; true when it used the key. */
	function typing(e: KeyboardEvent): boolean {
		if (e.metaKey || e.ctrlKey || e.altKey) return false;
		const printable = e.key.length === 1 && /[\p{L}\p{N} ]/u.test(e.key);
		if (book) {
			if (/^[0-9]$/.test(e.key)) {
				typeChapter(e.key);
				return true;
			}
			return false;
		}
		if (printable && (query || e.key !== ' ')) {
			void retype(query + e.key);
			return true;
		}
		if (!query) return false;
		if (e.key === 'Backspace') {
			void retype(query.slice(0, -1));
			return true;
		}
		if (e.key === 'Escape') {
			// The first escape clears the typing; the next closes.
			e.stopPropagation();
			void retype('');
			return true;
		}
		if (e.key === 'ArrowDown' || e.key === 'ArrowUp' || e.key === 'ArrowLeft' || e.key === 'ArrowRight') {
			// The arrows step through the matches, best first.
			if (matches.length) {
				const cell = Number(focused()?.dataset.cell);
				const at = Math.max(0, matches.indexOf(cell));
				const step = e.key === 'ArrowDown' || e.key === 'ArrowRight' ? 1 : -1;
				focusCell(String(matches[(at + step + matches.length) % matches.length]));
			}
			return true;
		}
		if (e.key === 'Enter') {
			// A chapter typed after the name ("rom 8") goes straight there.
			const r = /\p{L}.*\d/u.test(query) ? parseReference(query) : null;
			if (r) {
				app.dispatch({ type: 'jump', chapter: r.chapterIndex, verse: r.verse ?? undefined, flash: r.verse != null });
				onclose();
				return true;
			}
			const cell = focused()?.dataset.cell;
			const b = cell !== undefined ? books[Number(cell)] : undefined;
			if (b) chooseBook(b, true);
			return true;
		}
		return false;
	}

	function keydown(e: KeyboardEvent) {
		if (typing(e)) {
			e.preventDefault();
			return;
		}
		const dx = e.key === 'ArrowRight' ? 1 : e.key === 'ArrowLeft' ? -1 : 0;
		const dy = e.key === 'ArrowDown' ? 1 : e.key === 'ArrowUp' ? -1 : 0;
		const cell = focused()?.dataset.cell;
		if (dx || dy) {
			e.preventDefault();
			if (!book) {
				const from = cell !== undefined ? Number(cell) : (hereBook?.index ?? 0);
				const [col, row] = spot(from);
				const i = at(col + dx, row + dy);
				if (i !== null) focusCell(String(i));
				return;
			}
			const n = book.chapters;
			const current = hereBook?.index === book.index ? app.here.chapter - book.firstChapter : null;
			if (cell === 'back') {
				if (dy > 0) focusCell('0');
				return;
			}
			const from = cell !== undefined ? Number(cell) : current;
			if (from === null) {
				focusCell('0');
				return;
			}
			const j = from + dx + dy * CHAPTER_COLUMNS;
			if (j >= 0 && j < n) focusCell(String(j));
			else if (j < 0 && dy < 0) focusCell('back');
			return;
		}
		if (e.key === 'Enter' && cell === undefined) {
			e.preventDefault();
			if (!book && hereBook) chooseBook(hereBook, true);
			else if (book) chooseChapter(hereBook?.index === book.index ? app.here.chapter - book.firstChapter : 0);
		} else if (e.key === 'Backspace' && book) {
			e.preventDefault();
			backToBooks(true);
		}
	}

	/** The pointer and the keyboard share one highlight. */
	function pointerover(e: PointerEvent) {
		const el = (e.target as HTMLElement).closest<HTMLElement>('[data-cell]');
		if (el && document.activeElement !== el) el.focus({ preventScroll: true });
	}

	function pointerleave() {
		if (focused()) panel?.focus({ preventScroll: true });
	}

	// The keys reach the picker whether the panel or a cell has focus (the panel has it on opening).
	$effect(() => {
		const el = panel;
		if (!el) return;
		el.addEventListener('keydown', keydown);
		return () => el.removeEventListener('keydown', keydown);
	});

	// Cells are only reached by arrow keys; the panel holds focus between them.
	$effect(() => {
		void book;
		for (const c of cells()) c.tabIndex = -1;
	});
</script>

<Popover {anchor} {onclose} role="menu" label="Books" bind:panel class="book-picker">
	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<div class="body" onpointerover={pointerover} onpointerleave={pointerleave}>
		{#if !book}
			<div class="books">
				<p class="header" style:grid-column="1 / span 3">Old Testament</p>
				<p class="header" style:grid-column="5 / span 3">New Testament</p>
				{#if query}
					<p class="typed" style:grid-column="5 / span 3" aria-live="polite">
						{matches.length ? query : `${query}: no book`}
					</p>
				{/if}
				{#each books as b, i (b.index)}
					{@const [col, row] = spot(i)}
					<button
						type="button"
						class="cell"
						class:selected={b.index === hereBook?.index}
						class:muted={query !== '' && !matches.includes(b.index)}
						role="menuitem"
						data-cell={i}
						style:grid-column={col + 1 + (col >= COLUMNS ? 1 : 0)}
						style:grid-row={row + 2}
						onclick={(e) => chooseBook(b, e.detail === 0)}
					>
						{name(b)}
					</button>
				{/each}
			</div>
		{:else}
			{@const current = hereBook?.index === book.index ? app.here.chapter - book.firstChapter : -1}
			<div class="chapters">
				<div class="top">
					<button type="button" class="cell back" role="menuitem" data-cell="back" onclick={(e) => backToBooks(e.detail === 0)}>
						<Icon name="chevron-left" size={16} heavy />
						Books
					</button>
					<span class="title">{book.name}</span>
				</div>
				<div class="separator"></div>
				<div class="grid">
					{#each { length: book.chapters }, i (i)}
						<button
							type="button"
							class="cell number"
							class:selected={i === current}
							role="menuitem"
							data-cell={i}
							onclick={() => chooseChapter(i)}
						>
							{i + 1}
						</button>
					{/each}
				</div>
			</div>
		{/if}
	</div>
</Popover>

<style>
	.body {
		padding: var(--menu-padding-y) var(--menu-padding-x);
		outline: none;
	}

	/* One width for every book cell: the longest name with the text inset each side. Between the
	   testaments, twice the menu's side padding. */
	.books {
		display: grid;
		grid-template-columns: repeat(3, minmax(0, 1fr)) calc(2 * var(--menu-padding-x)) repeat(3, minmax(0, 1fr));
		grid-auto-rows: var(--menu-item-height);
		grid-template-rows: var(--menu-header-height);
		width: max-content;
		max-width: calc(100vw - 2 * (8px + var(--menu-padding-x)));
	}

	.header {
		grid-row: 1;
		margin: 0;
		padding: 6px var(--menu-item-padding) 0;
		font: var(--type-caption);
		color: var(--ink-3);
	}

	.cell {
		display: flex;
		align-items: center;
		min-width: 0;
		height: var(--menu-item-height);
		padding: 0 var(--menu-item-padding);
		border-radius: var(--menu-item-radius);
		font: var(--type-ui);
		color: var(--ink);
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
		outline: none;
	}

	.cell:focus {
		background: var(--fill);
	}

	.cell:active {
		background: var(--fill-strong);
	}

	.cell.selected {
		background: var(--selection);
	}

	.cell.muted {
		color: var(--ink-3);
	}

	/* What's typed, at the end of the headers' row. */
	.typed {
		grid-row: 1;
		justify-self: end;
		margin: 0;
		padding: 6px var(--menu-item-padding) 0;
		font: var(--type-caption);
		color: var(--ink-2);
		white-space: pre;
	}

	.top {
		display: flex;
		align-items: center;
	}

	.back {
		gap: var(--menu-icon-gap);
	}

	.back :global(svg) {
		color: var(--ink-2);
	}

	.title {
		margin-left: var(--menu-item-padding);
		font: var(--type-ui-title);
		letter-spacing: var(--type-ui-title-tracking);
	}

	.separator {
		height: var(--menu-separator-height);
		position: relative;
	}

	.separator::after {
		content: '';
		position: absolute;
		left: var(--menu-item-padding);
		right: var(--menu-item-padding);
		top: 4px;
		height: 1px;
		background: var(--separator);
	}

	.grid {
		display: grid;
		grid-template-columns: repeat(10, calc(3ch + 2 * var(--menu-item-padding)));
		font: var(--type-ui);
		font-variant-numeric: tabular-nums;
	}

	.number {
		justify-content: center;
		padding: 0;
	}
</style>
