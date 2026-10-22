<!--
	Where questions are typed: a field that grows with its text, Enter to ask (Shift-Enter for a new
	line), and Stop while a reply arrives. A new conversation shows what it will be about, which can
	be cleared to ask about anything.
-->
<script lang="ts">
	import { tick, untrack } from 'svelte';
	import { getApp } from '../app.svelte';
	import { Icon } from '../ui';
	import { contextIcon, contextLabel, type ChatContext } from './context';

	let { context, placeholder, wide = false }: { context: ChatContext; placeholder: string; wide?: boolean } = $props();

	const app = getApp();
	const chat = app.chat;
	const MAX_HEIGHT = 168;

	let text = $state('');
	let field = $state<HTMLTextAreaElement>();

	const busy = $derived(chat.streaming !== null);
	const fresh = $derived(!chat.active);

	function grow() {
		if (!field) return;
		field.style.height = 'auto';
		field.style.height = `${Math.min(MAX_HEIGHT, field.scrollHeight)}px`;
	}

	async function send() {
		const q = text.trim();
		if (!q || busy) return;
		text = '';
		await tick();
		grow();
		void chat.send(q);
	}

	function keydown(e: KeyboardEvent) {
		if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
			e.preventDefault();
			void send();
		} else if (e.key === 'Escape') {
			e.preventDefault();
			e.stopPropagation();
			if (busy) chat.stop();
			else field?.blur();
		}
	}

	// Asked for focus (the panel opening, a suggestion picked).
	let seen = untrack(() => chat.focusRequest);
	$effect(() => {
		const n = chat.focusRequest;
		if (n === seen) return;
		seen = n;
		void tick().then(() => field?.focus({ preventScroll: true }));
	});

	$effect(() => {
		if (app.demo['chat-type']) {
			text = app.demo['chat-type'];
			void tick().then(grow);
		}
	});
</script>

<div class="composer" class:wide>
	{#if fresh && context.kind !== 'general'}
		<div class="about">
			<span class="chip">
				<Icon name={contextIcon(context)} size={12} heavy />
				<span>{contextLabel(app, context)}</span>
				<button type="button" title="Ask about anything instead" aria-label="Ask about anything instead" onclick={() => (chat.draft = { kind: 'general' })}>
					<Icon name="close" size={10} heavy />
				</button>
			</span>
		</div>
	{/if}
	<div class="box">
		<textarea
			bind:this={field}
			bind:value={text}
			rows="1"
			{placeholder}
			aria-label="Ask a question"
			spellcheck="true"
			oninput={grow}
			onkeydown={keydown}
		></textarea>
		{#if busy}
			<button type="button" class="send stop" title="Stop" aria-label="Stop" onclick={() => chat.stop()}>
				<Icon name="stop" size={14} />
			</button>
		{:else}
			<button type="button" class="send" title="Ask (Enter)" aria-label="Ask" disabled={!text.trim()} onclick={send}>
				<Icon name="arrow-up" size={16} />
			</button>
		{/if}
	</div>
	<p class="hint">AI answers can be wrong. Check them against the passages shown.</p>
</div>

<style>
	.composer {
		flex-shrink: 0;
		padding: 10px 12px 10px;
		box-shadow: inset 0 1px 0 var(--separator);
	}

	.composer.wide > * {
		max-width: 720px;
		margin-left: auto;
		margin-right: auto;
	}

	.about {
		display: flex;
		margin-bottom: 8px;
	}

	.chip {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		height: 24px;
		padding: 0 4px 0 9px;
		border-radius: 12px;
		background: var(--accent-soft);
		color: var(--accent);
		font: 500 12px/16px var(--font-ui);
	}

	.chip button {
		display: grid;
		place-items: center;
		width: 18px;
		height: 18px;
		border-radius: 50%;
	}

	.chip button:hover {
		background: var(--fill-strong);
	}

	.box {
		display: flex;
		align-items: flex-end;
		gap: 8px;
		padding: 6px 6px 6px 12px;
		border-radius: 18px;
		background: var(--field);
		box-shadow: inset 0 0 0 1px var(--border);
		transition: box-shadow var(--motion-fast);
	}

	.box:focus-within {
		box-shadow:
			inset 0 0 0 1px var(--accent),
			0 0 0 3px var(--focus);
	}

	textarea {
		flex: 1;
		min-width: 0;
		height: 24px;
		max-height: 168px;
		margin: 0 0 1px;
		padding: 2px 0;
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

	.send {
		display: grid;
		flex-shrink: 0;
		place-items: center;
		width: 28px;
		height: 28px;
		border-radius: 50%;
		background: var(--accent);
		color: var(--on-accent);
		transition: opacity var(--motion-fast);
	}

	.send:disabled {
		opacity: 0.3;
	}

	.send:hover:not(:disabled) {
		background-image: linear-gradient(var(--hover-wash), var(--hover-wash));
	}

	.stop {
		background: var(--ink);
		color: var(--paper);
	}

	.hint {
		margin: 6px 4px 0;
		color: var(--ink-3);
		font: 400 11px/14px var(--font-ui);
		text-align: center;
	}
</style>
