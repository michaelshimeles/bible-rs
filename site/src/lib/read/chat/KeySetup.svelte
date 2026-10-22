<!--
	Adding an API key, in the chat panel: what it's for, where it's kept, a field, and a link to
	make one. The key is checked with the provider before it's saved.
-->
<script lang="ts">
	import { getApp } from '../app.svelte';
	import { Button, Icon, TextField } from '../ui';

	let { replacing = false, oncancel }: { replacing?: boolean; oncancel?: () => void } = $props();

	const app = getApp();
	const chat = app.chat;

	let key = $state('');
	let error = $state<string | null>(null);
	let checking = $state(false);

	async function save(e?: SubmitEvent) {
		e?.preventDefault();
		if (checking) return;
		checking = true;
		error = await chat.saveKey(key);
		checking = false;
		if (!error) {
			key = '';
			app.toastNow('API key saved in this browser');
		}
	}
</script>

<form class="setup" onsubmit={save}>
	<span class="glyph"><Icon name="key" size={20} heavy /></span>
	<h3 class="title">{replacing ? 'Replace your API key' : 'Bring your own OpenAI key'}</h3>
	<p class="body">
		Chat runs on your own OpenAI account: you pay OpenAI directly, usually well under a cent a question with the fast
		model.
	</p>
	<label class="label" for="chat-key">API key</label>
	<TextField
		id="chat-key"
		type="password"
		mono
		bind:value={key}
		error={!!error}
		placeholder={chat.provider.keyPlaceholder}
		autocomplete="off"
		data-autofocus
		oninput={() => (error = null)}
	/>
	{#if error}<p class="error" role="alert">{error}</p>{/if}
	<div class="buttons">
		<Button kind="primary" type="submit" disabled={checking || !key.trim()} label={checking ? 'Checking…' : 'Save key'} />
		{#if oncancel}<Button kind="plain" label="Cancel" onclick={oncancel} />{/if}
		<a class="get" href={chat.provider.keyUrl} target="_blank" rel="noopener noreferrer">
			Get a key <Icon name="external" size={12} heavy />
		</a>
	</div>
	<p class="privacy">
		<Icon name="lock" size={12} heavy />
		<span>Your key stays in this browser and is sent only to {chat.provider.name}. Conversations are kept here too.</span>
	</p>
</form>

<style>
	.setup {
		display: flex;
		flex-direction: column;
		gap: 8px;
		padding: 18px;
		border-radius: 14px;
		background: var(--surface);
		box-shadow:
			inset 0 0 0 1px var(--separator),
			var(--shadow-control);
	}

	.glyph {
		display: grid;
		place-items: center;
		width: 36px;
		height: 36px;
		margin-bottom: 4px;
		border-radius: 10px;
		background: var(--accent-soft);
		color: var(--accent);
	}

	.title {
		margin: 0;
		color: var(--ink);
		font: var(--type-ui-title);
		letter-spacing: var(--type-ui-title-tracking);
	}

	.body {
		margin: 0 0 6px;
		color: var(--ink-2);
		font: 400 13px/19px var(--font-ui);
	}

	.label {
		color: var(--ink-2);
		font: 500 12px/16px var(--font-ui);
	}

	.error {
		margin: 0;
		color: var(--danger);
		font: 400 12px/16px var(--font-ui);
	}

	.buttons {
		display: flex;
		align-items: center;
		gap: 8px;
		margin-top: 6px;
	}

	.get {
		display: inline-flex;
		align-items: center;
		gap: 4px;
		margin-left: auto;
		color: var(--accent);
		font: 500 13px/20px var(--font-ui);
		text-decoration: none;
	}

	.get:hover {
		text-decoration: underline;
	}

	.privacy {
		display: flex;
		gap: 6px;
		margin: 6px 0 0;
		color: var(--ink-3);
		font: 400 11px/15px var(--font-ui);
	}

	.privacy :global(svg) {
		margin-top: 1px;
	}
</style>
