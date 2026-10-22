<!--
	The Ask sheet: a question about a passage (or a chapter, book or note), opened from the verse
	menu, the palette or a page. It quotes what's asked about and offers starter questions; asking
	opens the chat panel with the reply streaming in.
-->
<script lang="ts">
	import { tick } from 'svelte';
	import { getApp } from '../app.svelte';
	import { Button, Icon, Sheet } from '../ui';
	import { contextIcon, contextKind, contextLabel, suggestions, type ChatContext } from './context';

	const app = getApp();
	const chat = app.chat;
	const context: ChatContext = chat.asking!;

	let text = $state(app.demo['ask-type'] ?? '');
	let field = $state<HTMLTextAreaElement>();

	const translation = $derived(app.settings.translation);
	const abbrev = $derived(app.info(translation)?.abbrev ?? '');
	const label = $derived(contextLabel(app, context));

	/** The passage quoted, verse numbers and all. */
	const verses = $derived.by(() => {
		if (!app.loaded.has(translation)) return [];
		const range =
			context.kind === 'verses'
				? [context.from, context.to]
				: context.kind === 'note'
					? [context.verse, context.verse]
					: context.kind === 'chapter'
						? [app.refAt(context.chapter, 1), app.refAt(context.chapter, 3)]
						: null;
		if (!range) return [];
		const [from, to] = range;
		const out: { n: number; text: string }[] = [];
		for (let v = from.verse; v <= to.verse && out.length < 5; v++) {
			const t = app.core?.verse(translation, { ...from, verse: v });
			if (t) out.push({ n: v, text: t.runs.map((r) => r.text).join('').trim() });
		}
		return out;
	});
	const more = $derived(
		context.kind === 'chapter' || (context.kind === 'verses' && context.to.verse - context.from.verse + 1 > verses.length)
	);

	function close() {
		chat.asking = null;
	}

	function ask(q = text) {
		if (!q.trim()) {
			field?.focus();
			return;
		}
		app.dispatch({ type: 'chatAbout', context, question: q });
	}

	function keydown(e: KeyboardEvent) {
		if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
			e.preventDefault();
			ask();
		}
	}

	$effect(() => {
		void tick().then(() => field?.setSelectionRange(text.length, text.length));
	});
</script>

<Sheet label="Ask about {label}" onclose={close} width={520} top={96} dismissable={!text.trim()}>
	<div class="ask">
		<header class="head">
			<span class="mark"><Icon name="sparkle" size={16} heavy /></span>
			<div class="titles">
				<h2 class="title">Ask about {label}</h2>
				<p class="detail"><Icon name={contextIcon(context)} size={12} heavy />{contextKind(context)} · {abbrev}</p>
			</div>
			<button type="button" class="close" aria-label="Close" title="Close" onclick={close}><Icon name="close" size={16} heavy /></button>
		</header>

		{#if verses.length}
			<blockquote class="quote">
				{#each verses as v (v.n)}<sup>{v.n}</sup>{v.text}{' '}{/each}{#if more}<span class="ellipsis">…</span>{/if}
			</blockquote>
		{/if}

		<div class="field">
			<textarea
				bind:this={field}
				bind:value={text}
				rows="3"
				data-autofocus
				placeholder={context.kind === 'verses' ? 'What would you like to know about this passage?' : `Ask about ${label}…`}
				aria-label="Your question"
				spellcheck="true"
				onkeydown={keydown}
			></textarea>
		</div>

		<div class="chips" aria-label="Suggested questions">
			{#each suggestions(context) as s (s)}
				<button type="button" class="chip" onclick={() => ask(s)}>{s}</button>
			{/each}
		</div>

		<footer class="foot">
			<span class="hint">
				{#if chat.hasKey}Enter to ask · Shift-Enter for a new line{:else}<Icon name="key" size={12} heavy />You’ll add your OpenAI key next{/if}
			</span>
			<Button kind="plain" label="Cancel" onclick={close} />
			<Button kind="primary" icon="sparkle" label="Ask" disabled={!text.trim()} onclick={() => ask()} />
		</footer>
	</div>
</Sheet>

<style>
	.ask {
		display: flex;
		flex-direction: column;
		gap: 14px;
		padding: 18px 20px 16px;
	}

	.head {
		display: flex;
		align-items: center;
		gap: 10px;
	}

	.mark {
		display: grid;
		place-items: center;
		width: 32px;
		height: 32px;
		border-radius: 9px;
		background: var(--accent);
		color: var(--on-accent);
	}

	.titles {
		flex: 1;
		min-width: 0;
	}

	.title {
		margin: 0;
		color: var(--ink);
		font: var(--type-ui-title);
		letter-spacing: var(--type-ui-title-tracking);
	}

	.detail {
		display: flex;
		align-items: center;
		gap: 4px;
		margin: 1px 0 0;
		color: var(--ink-3);
		font: var(--type-small);
	}

	.close {
		display: grid;
		place-items: center;
		width: 28px;
		height: 28px;
		border-radius: 50%;
		color: var(--ink-2);
	}

	.close:hover {
		background: var(--fill);
	}

	.quote {
		display: -webkit-box;
		margin: 0;
		padding: 2px 0 2px 14px;
		overflow: hidden;
		-webkit-box-orient: vertical;
		-webkit-line-clamp: 5;
		line-clamp: 5;
		box-shadow: inset 2px 0 var(--accent);
		color: var(--ink);
		font: 400 16px/25px var(--font-reading);
	}

	.quote sup {
		margin-right: 2px;
		color: var(--verse);
		font: 500 10px/1 var(--font-ui);
	}

	.ellipsis {
		color: var(--ink-3);
	}

	.field {
		padding: 10px 12px;
		border-radius: 12px;
		background: var(--field);
		box-shadow: inset 0 0 0 1px var(--border);
	}

	.field:focus-within {
		box-shadow:
			inset 0 0 0 1px var(--accent),
			0 0 0 3px var(--focus);
	}

	textarea {
		display: block;
		width: 100%;
		padding: 0;
		border: 0;
		outline: none;
		resize: none;
		background: none;
		color: var(--ink);
		caret-color: var(--accent);
		font: 400 14px/20px var(--font-ui);
	}

	textarea::placeholder {
		color: var(--ink-3);
	}

	.chips {
		display: flex;
		flex-wrap: wrap;
		gap: 6px;
	}

	.chip {
		height: 28px;
		padding: 0 11px;
		border-radius: 14px;
		background: var(--fill);
		color: var(--ink);
		font: 500 12px/16px var(--font-ui);
	}

	.chip:hover {
		background: var(--accent-soft);
		color: var(--accent);
	}

	.foot {
		display: flex;
		align-items: center;
		gap: 8px;
		margin-top: 2px;
	}

	.hint {
		display: flex;
		flex: 1;
		align-items: center;
		gap: 5px;
		color: var(--ink-3);
		font: 400 11px/14px var(--font-ui);
	}
</style>
