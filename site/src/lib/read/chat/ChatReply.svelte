<!--
	A reply in the chat: the tools it used, its text as Markdown with every Bible reference a link,
	and, once it's finished, the passages it cited as cards with their text from the reader's own
	translation (so what the model said can be checked against the Bible itself).
-->
<script lang="ts">
	import { getApp } from '../app.svelte';
	import { chapterPath } from '../books';
	import type { Reference, VerseRef } from '../core';
	import { Icon } from '../ui';
	import type { ChatMessage } from './chat.svelte';
	import type { ChatContext } from './context';
	import { parse, references, type Inline } from './markdown';

	interface Props {
		message: ChatMessage;
		translation: string;
		context: ChatContext;
		streaming: boolean;
		/** Following a reference (a phone closes the panel to show it). */
		onfollow?: () => void;
	}

	let { message, translation, context, streaming, onfollow }: Props = $props();

	const app = getApp();
	const CARDS = 3;
	const CARD_VERSES = 4;

	let allCards = $state(false);

	const books = $derived(app.core?.books ?? []);
	const blocks = $derived(parse(message.content, books));
	const abbrev = $derived(app.info(translation)?.abbrev ?? '');

	/** The cited passages, less the one the conversation is about. */
	const cited = $derived.by(() => {
		if (streaming) return [];
		return references(blocks).filter((r) => {
			if (context.kind === 'verses') {
				const i = app.chapterIndex(context.from);
				return !(r.chapterIndex === i && r.verse !== null && r.verse >= context.from.verse && (r.verseEnd ?? r.verse) <= context.to.verse);
			}
			if (context.kind === 'chapter') return !(r.chapterIndex === context.chapter && r.verse === null);
			return true;
		});
	});
	const shown = $derived(allCards ? cited : cited.slice(0, CARDS));

	function passage(r: Reference) {
		const verses = app.verses(translation, r.chapterIndex) ?? [];
		const from = r.verse ?? 1;
		const to = r.verse ? (r.verseEnd ?? r.verse) : from + 1;
		const picked = verses.filter((v) => v.numberEnd >= from && v.number <= to);
		const more = !r.verse || picked.length > CARD_VERSES;
		return {
			verses: picked.slice(0, CARD_VERSES).map((v) => ({
				n: v.number,
				runs: v.runs.map((x) => ({ text: x.text, red: x.red && app.settings.typo.redLetters }))
			})),
			more
		};
	}

	/** The chapter's page, so a reference is a real link (new tab, copy link). */
	function href(r: Reference): string {
		const core = app.core;
		if (!core) return '/read';
		const id = core.bundled.includes(translation) ? translation : (core.bundled[0] ?? 'kjv');
		return chapterPath(core.books, id, r.chapterIndex, r.verse ?? undefined);
	}

	function click(e: MouseEvent, r: Reference) {
		if (e.metaKey || e.ctrlKey || e.shiftKey || e.button !== 0) return;
		e.preventDefault();
		follow(r);
	}

	function follow(r: Reference) {
		app.dispatch({ type: 'jump', chapter: r.chapterIndex, verse: r.verse ?? undefined, flash: r.verse !== null });
		onfollow?.();
	}

	function refs(r: Reference): VerseRef[] {
		const out: VerseRef[] = [];
		const from = r.verse ?? 1;
		const to = r.verse ? (r.verseEnd ?? r.verse) : app.chapterVerses(r.chapterIndex);
		for (let v = from; v <= to; v++) out.push({ book: r.book, chapter: r.chapter, verse: v });
		return out;
	}

	async function copy(r: Reference) {
		const text = app.core?.copyText(translation, refs(r), 'reference');
		if (!text) return;
		try {
			await navigator.clipboard.writeText(text);
			app.toastNow(`Copied ${r.label}`);
		} catch {
			app.toastNow("Couldn't copy");
		}
	}

	function bookmark(r: Reference) {
		const v = { book: r.book, chapter: r.chapter, verse: r.verse ?? 1 };
		const on = app.study((c) => c.bible.toggleBookmark(v), "Couldn't save bookmark");
		if (on !== undefined) app.toastNow(`${on ? 'Bookmarked' : 'Removed bookmark'} ${app.label(v)}`);
	}

	const marked = (r: Reference) =>
		app.marks(app.settings.translation, r.chapterIndex).get(r.verse ?? 1)?.bookmark ?? false;
</script>

{#snippet inline(xs: Inline[])}
	{#each xs as x, i (i)}
		{#if x.t === 'text'}{x.text}{:else if x.t === 'bold'}<strong>{@render inline(x.children)}</strong>{:else if x.t === 'italic'}<em
				>{@render inline(x.children)}</em
			>{:else if x.t === 'code'}<code>{x.text}</code>{:else if x.t === 'ref'}<a class="ref" href={href(x.ref)} title="Go to {x.ref.label}" onclick={(e) => click(e, x.ref)}
				>{x.ref.label}</a
			>{/if}
	{/each}
{/snippet}

<div class="reply">
	{#if message.steps?.length}
		<ul class="steps" aria-label="What it looked up">
			{#each message.steps as s, i (i)}
				<li class={{ done: s.done }}>
					{#if s.done}<Icon name="check" size={12} heavy />{:else}<span class="spinner"></span>{/if}
					<span>{s.label}</span>
				</li>
			{/each}
		</ul>
	{/if}

	{#if message.content}
		<div class="text" class:live={streaming}>
			{#each blocks as b, i (i)}
				{#if b.kind === 'p'}<p>{@render inline(b.inlines)}</p>
				{:else if b.kind === 'h'}<p class="h">{@render inline(b.inlines)}</p>
				{:else if b.kind === 'quote'}<blockquote>{@render inline(b.inlines)}</blockquote>
				{:else if b.kind === 'ul'}
					<ul>
						{#each b.items as item, j (j)}<li>{@render inline(item)}</li>{/each}
					</ul>
				{:else if b.kind === 'ol'}
					<ol start={b.start}>
						{#each b.items as item, j (j)}<li>{@render inline(item)}</li>{/each}
					</ol>
				{/if}
			{/each}
		</div>
	{:else if streaming && !message.steps?.some((s) => !s.done)}
		<div class="typing" aria-label="Thinking"><span></span><span></span><span></span></div>
	{/if}

	{#if message.stopped}<p class="stopped">Stopped</p>{/if}

	{#if shown.length}
		<div class="cards">
			{#each shown as r (r.label + r.chapterIndex)}
				{@const p = passage(r)}
				<article class="card">
					<header class="card-head">
						<button type="button" class="card-ref" onclick={() => follow(r)}>{r.label}</button>
						<span class="card-abbrev">{abbrev}</span>
						<span class="card-actions">
							<button type="button" title="Copy {r.label}" aria-label="Copy {r.label}" onclick={() => copy(r)}>
								<Icon name="copy" size={14} heavy />
							</button>
							{#if r.verse}
								<button
									type="button"
									title={marked(r) ? 'Remove bookmark' : `Bookmark ${r.label}`}
									aria-label={marked(r) ? 'Remove bookmark' : `Bookmark ${r.label}`}
									onclick={() => bookmark(r)}
								>
									<Icon name={marked(r) ? 'bookmark-fill' : 'bookmark'} size={14} heavy />
								</button>
							{/if}
							<button type="button" title="Go to {r.label}" aria-label="Go to {r.label}" onclick={() => follow(r)}>
								<Icon name="arrow-right" size={14} heavy />
							</button>
						</span>
					</header>
					{#if p.verses.length}
						<p class="card-text">
							{#each p.verses as v (v.n)}<sup>{v.n}</sup>{#each v.runs as run, k (k)}{#if run.red}<span class="red"
											>{run.text}</span
										>{:else}{run.text}{/if}{/each}{' '}{/each}{#if p.more}<span class="more">…</span>{/if}
						</p>
					{:else}
						<p class="card-text missing">Not in the {abbrev}.</p>
					{/if}
				</article>
			{/each}
			{#if cited.length > CARDS && !allCards}
				<button type="button" class="all" onclick={() => (allCards = true)}>
					Show {cited.length - CARDS} more {cited.length - CARDS === 1 ? 'passage' : 'passages'}
				</button>
			{/if}
		</div>
	{/if}
</div>

<style>
	.reply {
		display: flex;
		flex-direction: column;
		gap: 10px;
		min-width: 0;
	}

	.steps {
		display: flex;
		flex-direction: column;
		gap: 4px;
		margin: 0;
		padding: 0;
		list-style: none;
		color: var(--ink-3);
		font: 400 12px/16px var(--font-ui);
	}

	.steps li {
		display: flex;
		align-items: center;
		gap: 6px;
	}

	.steps li.done :global(svg) {
		color: var(--success);
	}

	.spinner {
		width: 12px;
		height: 12px;
		border: 1.5px solid var(--border);
		border-top-color: var(--accent);
		border-radius: 50%;
		animation: spin 0.8s linear infinite;
	}

	@keyframes spin {
		to {
			transform: rotate(360deg);
		}
	}

	.text {
		color: var(--ink);
		font: 400 14px/22px var(--font-ui);
		overflow-wrap: anywhere;
	}

	.text > :first-child {
		margin-top: 0;
	}

	.text > :last-child {
		margin-bottom: 0;
	}

	.text p,
	.text ul,
	.text ol,
	.text blockquote {
		margin: 0 0 10px;
	}

	.text .h {
		margin: 14px 0 6px;
		font-weight: 600;
		font-size: 15px;
	}

	.text ul,
	.text ol {
		padding-left: 20px;
		list-style: disc;
	}

	.text ol {
		list-style: decimal;
	}

	.text li + li {
		margin-top: 4px;
	}

	.text li::marker {
		color: var(--ink-3);
	}

	.text blockquote {
		padding-left: 12px;
		box-shadow: inset 2px 0 var(--accent);
		color: var(--ink-2);
		font: italic 400 15px/23px var(--font-reading);
	}

	.text strong {
		font-weight: 600;
	}

	.text code {
		padding: 1px 4px;
		border-radius: 4px;
		background: var(--fill);
		font: 12.5px/1 ui-monospace, SFMono-Regular, Menlo, monospace;
	}

	/* The caret at the end of a reply that's still arriving. */
	.text.live > :last-child::after {
		content: '';
		display: inline-block;
		width: 7px;
		height: 14px;
		margin-left: 2px;
		vertical-align: -2px;
		border-radius: 1px;
		background: var(--accent);
		animation: blink 1s steps(2, start) infinite;
	}

	.text.live > ul:last-child::after,
	.text.live > ol:last-child::after {
		display: none;
	}

	.text.live > ul:last-child > li:last-child::after,
	.text.live > ol:last-child > li:last-child::after {
		content: '';
		display: inline-block;
		width: 7px;
		height: 14px;
		margin-left: 2px;
		vertical-align: -2px;
		border-radius: 1px;
		background: var(--accent);
		animation: blink 1s steps(2, start) infinite;
	}

	@keyframes blink {
		to {
			visibility: hidden;
		}
	}

	.ref {
		display: inline;
		padding: 0 3px;
		margin: 0 -1px;
		border-radius: 4px;
		background: var(--accent-soft);
		color: var(--accent);
		font: inherit;
		font-weight: 600;
		text-decoration: none;
		cursor: pointer;
		box-decoration-break: clone;
		-webkit-box-decoration-break: clone;
	}

	.ref:hover {
		text-decoration: underline;
		text-underline-offset: 2px;
	}

	.typing {
		display: flex;
		gap: 4px;
		height: 22px;
		align-items: center;
	}

	.typing span {
		width: 6px;
		height: 6px;
		border-radius: 50%;
		background: var(--ink-3);
		animation: bounce 1.2s ease-in-out infinite;
	}

	.typing span:nth-child(2) {
		animation-delay: 0.15s;
	}

	.typing span:nth-child(3) {
		animation-delay: 0.3s;
	}

	@keyframes bounce {
		0%,
		60%,
		100% {
			opacity: 0.35;
			transform: none;
		}
		30% {
			opacity: 1;
			transform: translateY(-3px);
		}
	}

	.stopped {
		margin: 0;
		color: var(--ink-3);
		font: var(--type-small);
	}

	.cards {
		display: flex;
		flex-direction: column;
		gap: 8px;
		margin-top: 2px;
	}

	.card {
		padding: 8px 10px 10px 12px;
		border-radius: 12px;
		background: var(--paper);
		box-shadow: inset 0 0 0 1px var(--separator);
	}

	.card-head {
		display: flex;
		align-items: center;
		gap: 6px;
		height: 20px;
	}

	.card-ref {
		color: var(--accent);
		font: 600 13px/16px var(--font-ui);
	}

	.card-ref:hover {
		text-decoration: underline;
		text-underline-offset: 2px;
	}

	.card-abbrev {
		color: var(--ink-3);
		font: 500 11px/16px var(--font-ui);
	}

	.card-actions {
		display: flex;
		gap: 2px;
		margin-left: auto;
	}

	.card-actions button {
		display: grid;
		place-items: center;
		width: 24px;
		height: 24px;
		border-radius: 50%;
		color: var(--ink-3);
	}

	.card-actions button:hover {
		background: var(--fill);
		color: var(--ink);
	}

	.card-text {
		display: -webkit-box;
		margin: 4px 0 0;
		overflow: hidden;
		-webkit-box-orient: vertical;
		-webkit-line-clamp: 3;
		line-clamp: 3;
		color: var(--ink);
		font: 400 14px/21px var(--font-reading);
	}

	.card-text sup {
		margin-right: 2px;
		color: var(--verse);
		font: 500 10px/1 var(--font-ui);
	}

	.card-text .red {
		color: var(--rubric);
	}

	.card-text.missing,
	.more {
		color: var(--ink-3);
	}

	.all {
		align-self: flex-start;
		padding: 2px 0;
		color: var(--accent);
		font: 500 12px/16px var(--font-ui);
	}

	.all:hover {
		text-decoration: underline;
	}
</style>
