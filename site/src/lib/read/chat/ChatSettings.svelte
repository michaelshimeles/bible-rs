<!--
	The chat's settings, in the panel: the API key, the model, and the conversations kept in this
	browser.
-->
<script lang="ts">
	import { getApp } from '../app.svelte';
	import { Button, Icon, Radio, TextField } from '../ui';
	import KeySetup from './KeySetup.svelte';

	const app = getApp();
	const chat = app.chat;

	let replacing = $state(false);
	let confirmClear = $state(false);

	const models = $derived(chat.provider.models);
	const custom = $derived(!models.some((m) => m.id === chat.config.model));
	let customModel = $state(chat.config.model);
	let customOn = $state(false);

	const masked = $derived.by(() => {
		const k = chat.config.key;
		return k.length > 10 ? `${k.slice(0, 3)}…${k.slice(-4)}` : '••••';
	});
</script>

<div class="settings">
	<section>
		<h3 class="head">API key</h3>
		{#if !chat.hasKey || replacing}
			<KeySetup {replacing} oncancel={replacing ? () => (replacing = false) : undefined} />
		{:else}
			<div class="key-row">
				<span class="glyph"><Icon name="key" size={16} heavy /></span>
				<div class="key-text">
					<span class="key-name">{chat.provider.name}</span>
					<span class="key-value">{masked}</span>
				</div>
				<Button size="sm" label="Replace" onclick={() => (replacing = true)} />
				<Button
					size="sm"
					kind="plain"
					label="Remove"
					onclick={() => {
						chat.removeKey();
						app.toastNow('API key removed from this browser');
					}}
				/>
			</div>
			<p class="note">Stored only in this browser. Requests go straight to {chat.provider.name}.</p>
		{/if}
	</section>

	<section>
		<h3 class="head">Model</h3>
		<div class="models" role="radiogroup" aria-label="Model">
			{#each models as m (m.id)}
				{@const pick = () => {
					customOn = false;
					chat.setModel(m.id);
				}}
				<div class="model">
					<Radio bare name="chat-model" checked={!customOn && chat.config.model === m.id} onchange={pick} label={m.label} />
					<button type="button" class="model-text" onclick={pick}>
						<span class="model-name">{m.label}{#if m.id === chat.provider.defaultModel}<span class="default">&nbsp;· Default</span>{/if}</span>
						<span class="model-detail">{m.detail}</span>
					</button>
				</div>
			{/each}
			<div class="model">
				<Radio bare name="chat-model" checked={customOn || custom} onchange={() => (customOn = true)} label="Another model" />
				<div class="model-text">
					<button type="button" class="model-name" onclick={() => (customOn = true)}>Another model</button>
					{#if customOn || custom}
						<TextField
							mono
							bind:value={customModel}
							placeholder="gpt-4o-mini"
							aria-label="Model id"
							onchange={() => chat.setModel(customModel)}
							onkeydown={(e) => e.key === 'Enter' && chat.setModel(customModel)}
						/>
					{:else}
						<span class="model-detail">Any chat model your key can use</span>
					{/if}
				</div>
			</div>
		</div>
	</section>

	<section>
		<h3 class="head">History</h3>
		<div class="key-row">
			<span class="glyph"><Icon name="history" size={16} heavy /></span>
			<div class="key-text">
				<span class="key-name">{chat.conversations.length === 1 ? '1 conversation' : `${chat.conversations.length} conversations`}</span>
				<span class="key-value">Kept in this browser</span>
			</div>
			{#if confirmClear}
				<Button
					size="sm"
					kind="destructive"
					label="Delete all"
					onclick={() => {
						chat.clearHistory();
						confirmClear = false;
						app.toastNow('Chat history deleted');
					}}
				/>
				<Button size="sm" kind="plain" label="Cancel" onclick={() => (confirmClear = false)} />
			{:else}
				<Button size="sm" kind="plain" label="Clear…" disabled={!chat.conversations.length} onclick={() => (confirmClear = true)} />
			{/if}
		</div>
	</section>

	<p class="caution">
		<Icon name="info" size={12} heavy />
		<span>Answers come from an AI model and can be wrong. The cards under each answer show the passages from your own translation, so you can check them.</span>
	</p>
</div>

<style>
	.settings {
		display: flex;
		flex-direction: column;
		gap: 20px;
		padding: 16px;
	}

	section {
		display: flex;
		flex-direction: column;
		gap: 8px;
	}

	.head {
		margin: 0;
		color: var(--ink-3);
		font: 600 11px/16px var(--font-ui);
		letter-spacing: 0.04em;
		text-transform: uppercase;
	}

	.key-row {
		display: flex;
		align-items: center;
		gap: 10px;
		padding: 10px 12px;
		border-radius: 12px;
		background: var(--surface);
		box-shadow: inset 0 0 0 1px var(--separator);
	}

	.glyph {
		display: grid;
		place-items: center;
		width: 28px;
		height: 28px;
		border-radius: 8px;
		background: var(--accent-soft);
		color: var(--accent);
	}

	.key-text {
		display: flex;
		flex: 1;
		flex-direction: column;
		min-width: 0;
	}

	.key-name {
		color: var(--ink);
		font: var(--type-ui-strong);
	}

	.key-value {
		overflow: hidden;
		color: var(--ink-3);
		font: 400 12px/16px var(--font-ui);
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.note {
		margin: 0;
		color: var(--ink-3);
		font: 400 11px/15px var(--font-ui);
	}

	.models {
		display: flex;
		flex-direction: column;
		border-radius: 12px;
		background: var(--surface);
		box-shadow: inset 0 0 0 1px var(--separator);
	}

	.model {
		display: flex;
		align-items: flex-start;
		gap: 10px;
		padding: 10px 12px;
	}

	.model + .model {
		box-shadow: inset 0 1px var(--separator);
	}

	.model > :global(.radio) {
		margin-top: 1px;
	}

	.model-text {
		display: flex;
		align-items: flex-start;
		text-align: left;
		flex: 1;
		flex-direction: column;
		gap: 4px;
		min-width: 0;
	}

	.model-name {
		color: var(--ink);
		font: 500 13px/18px var(--font-ui);
	}

	.default,
	.model-detail {
		color: var(--ink-3);
		font: 400 12px/16px var(--font-ui);
	}

	.caution {
		display: flex;
		gap: 6px;
		margin: 0;
		color: var(--ink-3);
		font: 400 11px/15px var(--font-ui);
	}

	.caution :global(svg) {
		margin-top: 1px;
	}
</style>
