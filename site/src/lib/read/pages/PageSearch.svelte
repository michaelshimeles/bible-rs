<!--
	The field that filters a page (pages/mod.rs `filter_field`), drawn over the list it filters
	rather than in the toolbar, so it isn't mistaken for the main search. It takes the field
	colour, not the toolbar's glass, and shows a / keycap while it's empty: / focuses it.
-->
<script lang="ts">
	import { getApp } from '../app.svelte';
	import { Keycap, SearchField } from '../ui';

	let { placeholder, width = 280 }: { placeholder: string; width?: number | string } = $props();

	const app = getApp();
	let input = $state<HTMLInputElement>();
	let focused = $state(false);

	$effect(() => {
		const el = input;
		if (!el) return;
		app.pageField = el;
		return () => {
			if (app.pageField === el) app.pageField = null;
		};
	});

	function keydown(e: KeyboardEvent) {
		if (e.key === 'Escape') {
			e.preventDefault();
			e.stopPropagation();
			app.pageQuery = '';
			input?.blur();
		}
	}
</script>

<div class="page-search" style:width={typeof width === 'number' ? `${width}px` : width}>
	<SearchField
		glass={false}
		width="100%"
		{placeholder}
		aria-label={placeholder}
		aria-keyshortcuts="/"
		bind:value={app.pageQuery}
		bind:input
		onkeydown={keydown}
		onfocus={() => (focused = true)}
		onblur={() => (focused = false)}
	/>
	{#if !app.pageQuery && !focused}
		<Keycap label="/" class="page-search-key" />
	{/if}
</div>

<style>
	.page-search {
		position: relative;
		max-width: 100%;
		margin-bottom: var(--page-search-gap, 16px);
	}

	.page-search :global(.page-search-key) {
		position: absolute;
		top: 50%;
		right: 8px;
		transform: translateY(-50%);
		pointer-events: none;
	}
</style>
