<!--
	Past conversations, newest first, kept in this browser: each with what it was about and when.
	A filter finds one by its words; each can be deleted.
-->
<script lang="ts">
	import { getApp } from '../app.svelte';
	import { ago } from '../time';
	import { EmptyState, Icon, SearchField } from '../ui';
	import { contextIcon, contextLabel } from './context';

	const app = getApp();
	const chat = app.chat;

	let query = $state('');

	const list = $derived.by(() => {
		const q = query.trim().toLowerCase();
		return [...chat.conversations]
			.sort((a, b) => b.updated - a.updated)
			.filter(
				(c) =>
					!q ||
					c.title.toLowerCase().includes(q) ||
					contextLabel(app, c.context).toLowerCase().includes(q) ||
					c.messages.some((m) => m.content.toLowerCase().includes(q))
			);
	});
</script>

<div class="history">
	{#if chat.conversations.length}
		<div class="filter">
			<SearchField glass={false} width="100%" placeholder="Search conversations" aria-label="Search conversations" bind:value={query} />
		</div>
	{/if}
	<ul class="list">
		{#each list as c (c.id)}
			<li class={['item', { on: c.id === chat.activeId }]}>
				<button type="button" class="open" onclick={() => chat.select(c.id)}>
					<span class="glyph"><Icon name={contextIcon(c.context)} size={14} heavy /></span>
					<span class="text">
						<span class="title">{c.title}</span>
						<span class="meta"
							>{contextLabel(app, c.context)} · {Math.ceil(c.messages.length / 2)}
							{Math.ceil(c.messages.length / 2) === 1 ? 'question' : 'questions'} · {ago(Math.floor(c.updated / 1000))}</span
						>
					</span>
				</button>
				<button type="button" class="delete" title="Delete conversation" aria-label="Delete conversation" onclick={() => chat.remove(c.id)}>
					<Icon name="trash" size={14} heavy />
				</button>
			</li>
		{/each}
	</ul>
	{#if !chat.conversations.length}
		<EmptyState
			glyph="chat"
			tone="accent"
			title="No conversations yet"
			body="Your conversations are saved in this browser so you can come back to them."
			width={260}
		/>
	{:else if !list.length}
		<p class="none">Nothing matches “{query}”.</p>
	{/if}
</div>

<style>
	.history {
		display: flex;
		flex-direction: column;
		padding: 12px 8px 16px;
	}

	.filter {
		padding: 0 8px 10px;
	}

	.list {
		display: flex;
		flex-direction: column;
		gap: 2px;
		margin: 0;
		padding: 0;
		list-style: none;
	}

	.item {
		position: relative;
		display: flex;
		align-items: center;
		border-radius: 10px;
	}

	.item:hover {
		background: var(--fill);
	}

	.item.on {
		background: var(--accent-soft);
	}

	.open {
		display: flex;
		flex: 1;
		align-items: flex-start;
		gap: 10px;
		min-width: 0;
		padding: 9px 8px 9px 10px;
		text-align: left;
	}

	.glyph {
		display: grid;
		flex-shrink: 0;
		place-items: center;
		width: 24px;
		height: 24px;
		border-radius: 7px;
		background: var(--raised);
		box-shadow: var(--shadow-control);
		color: var(--accent);
	}

	.text {
		display: flex;
		flex-direction: column;
		min-width: 0;
	}

	.title,
	.meta {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.title {
		color: var(--ink);
		font: 500 13px/18px var(--font-ui);
	}

	.meta {
		color: var(--ink-3);
		font: 400 11px/16px var(--font-ui);
	}

	.delete {
		display: grid;
		place-items: center;
		width: 28px;
		height: 28px;
		margin-right: 6px;
		border-radius: 50%;
		color: var(--ink-3);
		opacity: 0;
	}

	.item:hover .delete,
	.delete:focus-visible {
		opacity: 1;
	}

	.delete:hover {
		background: var(--fill-strong);
		color: var(--danger);
	}

	@media (hover: none) {
		.delete {
			opacity: 1;
		}
	}

	.none {
		margin: 16px;
		color: var(--ink-3);
		font: var(--type-small);
		text-align: center;
	}
</style>
