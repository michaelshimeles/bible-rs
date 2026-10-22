<!--
	The keyboard shortcuts (shortcuts_overlay.rs, `?`, ⌘/ or the toolbar's keyboard button): every
	shortcut by group in three columns with keycaps, a filter, and a way to change them in Settings.
-->
<script lang="ts">
	import { getApp } from '../app.svelte';
	import { COLUMNS, SHORTCUTS, cmdText, rowCaps } from '../keyboard';
	import { Button, Keycap, RoundButton, Sheet, TextField } from '../ui';

	const app = getApp();

	let filter = $state('');

	/** Each column's groups, without those the filter empties. */
	const columns = $derived.by(() => {
		const q = filter.trim().toLowerCase();
		return COLUMNS.map((groups) =>
			groups
				.map((group) => ({
					group,
					rows: SHORTCUTS.filter((s) => s.group === group && (!q || s.label.toLowerCase().includes(q)))
				}))
				.filter((g) => g.rows.length)
		);
	});
	/** The body keeps its height while filtering. */
	const tallest = $derived(
		Math.max(...COLUMNS.map((groups) => groups.reduce((n, g) => n + SHORTCUTS.filter((s) => s.group === g).length, 0)))
	);
	const sections = $derived(Math.max(...COLUMNS.map((g) => g.length)));

	const close = () => (app.shortcuts = false);
</script>

<Sheet label="Keyboard Shortcuts" onclose={close} width={960} top={72} focusSheet class="shortcuts-sheet">
	<header class="head">
		<h2 class="title">Keyboard Shortcuts</h2>
		<TextField bind:value={filter} icon="search" placeholder="Filter" aria-label="Filter shortcuts" class="filter" />
		<RoundButton icon="close" label="Close" onclick={close} />
	</header>
	<div class="body" style:--rows={tallest} style:--sections={sections}>
		{#if columns.every((c) => c.length === 0)}
			<p class="none">No shortcut matches.</p>
		{:else}
			{#each columns as groups, i (i)}
				<div class="column">
					{#each groups as { group, rows } (group)}
						<section class="section" aria-label={group}>
							<h3 class="group">{group}</h3>
							{#each rows as s (s.id)}
								<div class="row">
									<span class="label">{s.label}</span>
									<span class="caps">{#each rowCaps(s, app.settings.shortcuts) as c, i (i)}<Keycap label={c} />{/each}</span>
								</div>
							{/each}
						</section>
					{/each}
				</div>
			{/each}
		{/if}
	</div>
	<footer class="foot">
		<p class="tip">Press ? or {cmdText('/')} any time to see this.</p>
		<Button
			kind="ghost"
			size="sm"
			icon="arrow-right"
			trailing
			label="Customize in Settings"
			onclick={() => {
				close();
				app.dispatch({ type: 'openSettings', tab: 'shortcuts' });
			}}
		/>
	</footer>
</Sheet>

<style>
	.head {
		display: flex;
		align-items: center;
		height: 60px;
		padding: 0 16px 0 24px;
		box-shadow: inset 0 -1px 0 var(--separator);
	}

	.title {
		flex: 1 0 auto;
		margin: 0;
		white-space: nowrap;
		color: var(--ink);
		font: 600 15px/20px var(--font-ui);
	}

	.head :global(.filter) {
		flex: 0 1 200px;
		min-width: 0;
		margin: 0 12px;
	}

	/* Under the toolbar, and clear of the window's bottom edge. */
	:global(.layer .sheet.shortcuts-sheet) {
		max-height: calc(100dvh - 72px - 16px);
	}

	.body {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
		gap: 16px 32px;
		align-content: start;
		min-height: min(calc(20px + (21px + 16px) * var(--sections) + 30px * var(--rows)), 60dvh);
		padding: 20px 24px 8px;
		overflow-y: auto;
	}

	.section + .section {
		margin-top: 16px;
	}

	.group {
		margin: 0 0 5px;
		color: var(--accent);
		font: 600 11px/16px var(--font-ui);
		letter-spacing: 0.12em;
		text-transform: uppercase;
	}

	.row {
		display: flex;
		align-items: center;
		justify-content: space-between;
		height: 30px;
		gap: 12px;
	}

	.label {
		color: var(--ink);
		font: var(--type-ui);
	}

	.caps {
		display: flex;
		gap: 4px;
	}

	.none {
		margin: 0;
		color: var(--ink-3);
		font: var(--type-ui);
	}

	.foot {
		display: flex;
		align-items: center;
		justify-content: space-between;
		flex-shrink: 0;
		height: 44px;
		padding: 0 16px 0 24px;
		background: var(--fill);
		box-shadow: inset 0 1px 0 var(--separator);
	}

	/* A phone has no keyboard to press it with. */
	@media (max-width: 520px) {
		.tip {
			display: none;
		}
	}

	.tip {
		margin: 0;
		color: var(--ink-3);
		font: 400 12px/16px var(--font-ui);
	}
</style>
