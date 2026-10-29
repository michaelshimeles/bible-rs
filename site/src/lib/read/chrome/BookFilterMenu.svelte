<!--
	The menu under an "All books" select (crates/app/src/book_filter.rs): the search sidebar's scope
	and Study › Highlights. A field to type a book's name and the books below it. Typing narrows the
	list, reading names the way the jump palette does ("jn", "1co", "song", "revalation"); ↑ ↓ move
	the highlight, return chooses it and escape closes, so a book you know is a few keys away.
-->
<script lang="ts">
	import { tick } from 'svelte';
	import { getApp } from '../app.svelte';
	import { filterBooks } from '../core';
	import { MenuSeparator, Popover, TextField } from '../ui';

	interface Props {
		anchor: HTMLElement;
		/** The current choice (null: all books). */
		current: number | null;
		/** The book being read, offered as "Name (this book)" under "All books". */
		here?: number | null;
		/** The books listed before anything is typed. */
		listed: number[];
		/** The books typing can find (the listed ones unless given). */
		searchable?: number[];
		width?: number;
		label?: string;
		onchoose: (book: number | null) => void;
		onclose: () => void;
	}

	let {
		anchor,
		current,
		here = null,
		listed,
		searchable,
		width = 200,
		label = 'Books',
		onchoose,
		onclose
	}: Props = $props();

	const app = getApp();
	const uid = $props.id();

	interface Row {
		book: number | null;
		label: string;
		selected: boolean;
	}

	let query = $state('');
	/** The lit row; null follows the default (the current choice, or the best match). */
	let active = $state<number | null>(null);
	let list = $state<HTMLElement>();

	const name = (b: number) => app.core?.books[b]?.name ?? '';
	const typing = $derived(query.trim() !== '');

	const rows = $derived.by((): Row[] => {
		const row = (b: number, label = name(b)): Row => ({ book: b, label, selected: current === b });
		if (!typing) {
			const out: Row[] = [{ book: null, label: 'All books', selected: current === null }];
			if (here !== null && app.core?.books[here]) out.push(row(here, `${name(here)} (this book)`));
			return out.concat(listed.map((b) => row(b)));
		}
		const pool = new Set(searchable ?? listed);
		return filterBooks(query)
			.filter((b) => pool.has(b))
			.map((b) => row(b));
	});
	/** Rows before the separator (none while typing). */
	const head = $derived(typing ? 0 : here !== null && app.core?.books[here] ? 2 : 1);
	const lit = $derived(
		Math.min(active ?? (typing ? 0 : Math.max(0, rows.findIndex((r) => r.selected))), rows.length - 1)
	);

	// Typing lights the best match again.
	$effect(() => {
		void query;
		active = null;
	});

	// The lit row stays in view as the keyboard moves it, and the current one shows on opening.
	$effect(() => {
		const i = lit;
		void tick().then(() =>
			list?.querySelector<HTMLElement>(`[data-row="${i}"]`)?.scrollIntoView({ block: 'nearest' })
		);
	});

	function choose(r: Row | undefined) {
		if (!r) return;
		onchoose(r.book);
		onclose();
	}

	function keydown(e: KeyboardEvent) {
		const n = rows.length;
		if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
			e.preventDefault();
			if (n) active = (lit + (e.key === 'ArrowDown' ? 1 : -1) + n) % n;
		} else if (e.key === 'Enter') {
			e.preventDefault();
			choose(rows[lit]);
		} else if (e.key === 'Tab') {
			e.preventDefault();
			onclose();
		}
	}
</script>

{#snippet item(r: Row, i: number)}
	<!-- svelte-ignore a11y_click_events_have_key_events -->
	<div
		id="{uid}-{i}"
		class={['item', { lit: i === lit, selected: r.selected }]}
		role="option"
		tabindex="-1"
		aria-selected={r.selected}
		data-row={i}
		onpointermove={() => (active = i)}
		onpointerdown={(e) => e.preventDefault()}
		onclick={() => choose(r)}
	>
		{r.label}
	</div>
{/snippet}

<Popover {anchor} {onclose} {label} class="book-filter">
	<div class="body" style:width="{width}px">
		<div class="find">
			<TextField
				bind:value={query}
				icon="search"
				placeholder="Type a book"
				aria-label="Find a book"
				role="combobox"
				aria-expanded="true"
				aria-controls="{uid}-list"
				aria-activedescendant={rows.length ? `${uid}-${lit}` : undefined}
				aria-autocomplete="list"
				autocomplete="off"
				onkeydown={keydown}
			/>
		</div>
		<div id="{uid}-list" role="listbox" aria-label={label}>
			{#each rows.slice(0, head) as r, i (i)}
				{@render item(r, i)}
			{/each}
			{#if head}<MenuSeparator />{/if}
			<div class="books" bind:this={list}>
				{#each rows.slice(head) as r, k (r.book ?? -1)}
					{@render item(r, head + k)}
				{:else}
					<p class="none">No books match</p>
				{/each}
			</div>
		</div>
	</div>
</Popover>

<style>
	.body {
		display: flex;
		flex-direction: column;
		padding: var(--menu-padding-y) var(--menu-padding-x);
	}

	.find {
		margin-bottom: 4px;
	}

	.find :global(.field) {
		height: 28px;
	}

	.books {
		max-height: 280px;
		overflow-y: auto;
		overscroll-behavior: contain;
	}

	.item,
	.none {
		display: flex;
		align-items: center;
		height: var(--menu-item-height);
		margin: 0;
		padding: 0 var(--menu-item-padding);
		border-radius: var(--menu-item-radius);
		font: var(--type-ui);
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}

	.item {
		color: var(--ink);
		cursor: default;
	}

	.item.lit {
		background: var(--fill);
	}

	.item.lit:active {
		background: var(--fill-strong);
	}

	.item.selected {
		background: var(--selection);
	}

	.none {
		color: var(--ink-3);
	}
</style>
