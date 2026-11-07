<!--
	The custom-plan editor (pages/plans.rs `editor_view`): a title, optional description, typed
	books and passages to fill days at a chosen pace, then each day's references to edit, reorder,
	or remove.
-->
<script lang="ts">
	import { fillPlanDays } from '../core';
	import { getApp } from '../app.svelte';
	import { Button, Icon, RoundButton, Segmented, TextField } from '../ui';
	import { plansPage, type PlanEditor } from './plans.svelte';

	const app = getApp();

	let editor = $derived(plansPage.editor as PlanEditor);

	const PACE = [
		{ value: 0, label: '1 a day' },
		{ value: 1, label: '2 a day' },
		{ value: 2, label: '3 a day' },
		{ value: 3, label: 'In N days' }
	];

	const creating = $derived(editor.id === null);
	const crumb = $derived(creating ? 'New plan' : 'Edit plan');

	function close() {
		plansPage.editor = null;
		plansPage.tab = 'mine';
	}

	function fill() {
		try {
			const chapters = editor.paceIdx < 3 ? editor.paceIdx + 1 : 0;
			const inDays = Number(editor.inDays) || 0;
			const days = fillPlanDays(editor.source, chapters, inDays);
			editor.days = days.length ? days : [''];
			editor.error = null;
		} catch (e) {
			editor.error = e instanceof Error ? e.message : String(e);
		}
	}

	function save() {
		const title = editor.title.trim();
		if (!title) {
			editor.error = 'Give the plan a title';
			return;
		}
		if ([...title].length > 120) {
			editor.error = 'That title is too long';
			return;
		}
		if (!editor.days.some((d) => d.trim())) {
			editor.error = 'Add at least one day with a passage';
			return;
		}
		const gen = app.studyGen;
		app.dispatch({
			type: 'saveCustomPlan',
			id: editor.id ?? '',
			title,
			description: editor.description.trim(),
			days: editor.days
		});
		if (app.studyGen !== gen) close();
	}

	function removeDay(i: number) {
		editor.days.splice(i, 1);
		if (!editor.days.length) editor.days.push('');
	}

	function deletePlan() {
		if (!editor.id) return;
		app.dispatch({ type: 'deleteCustomPlan', id: editor.id });
		close();
	}
</script>

<nav class="crumb" aria-label="Breadcrumb">
	<button type="button" onclick={close}>Reading Plans</button>
	<Icon name="disclosure" size={12} />
	<span>{crumb}</span>
</nav>

<div class="form">
	<label class="named">
		<span class="label">Title</span>
		<TextField bind:value={editor.title} placeholder="Wisdom in a month" data-autofocus />
	</label>
	<label class="named">
		<span class="label">Description</span>
		<TextField bind:value={editor.description} placeholder="Optional — what this plan is for" />
	</label>
	<h2 class="section">Build a schedule</h2>
	<p class="hint">Books, chapters, or verse ranges — “John, Matthew 5–7, Proverbs 3:5–6”.</p>
	<label class="named">
		<TextField bind:value={editor.source} placeholder="John, Matthew 5–7" />
	</label>
	<div class="pace">
		<span class="label">Pace</span>
		<Segmented options={PACE} bind:value={editor.paceIdx} layout="tight" label="Pace" />
		{#if editor.paceIdx === 3}
			<TextField bind:value={editor.inDays} placeholder="30" class="in-days" inputmode="numeric" />
		{/if}
	</div>
	<Button kind="secondary" label="Fill days" disabled={!editor.source.trim()} onclick={fill} />
	<h2 class="days-head">Days ({editor.days.length})</h2>
	<ul class="days">
		{#each editor.days as _day, i (i)}
			<li class="day">
				<span class="num">{i + 1}</span>
				<TextField bind:value={editor.days[i]} placeholder="John 1 · Proverbs 3" />
				{#if i > 0}
					<RoundButton
						icon="chevron-up"
						label="Move up"
						onclick={() => editor.days.splice(i - 1, 2, editor.days[i], editor.days[i - 1])}
					/>
				{/if}
				{#if i + 1 < editor.days.length}
					<RoundButton
						icon="chevron-down"
						label="Move down"
						onclick={() => editor.days.splice(i, 2, editor.days[i + 1], editor.days[i])}
					/>
				{/if}
				<RoundButton icon="trash" label="Remove day" onclick={() => removeDay(i)} />
			</li>
		{/each}
	</ul>
	<Button kind="plain" icon="plus" label="Add day" onclick={() => editor.days.push('')} />
	{#if editor.error}
		<p class="error">{editor.error}</p>
	{/if}
	<div class="actions">
		<Button kind="plain" label="Cancel" onclick={close} />
		<Button kind="primary" label="Save plan" onclick={save} />
		{#if editor.id}
			{#if editor.confirmDelete}
				<p class="warn">This deletes the plan. If you have started it, its progress is removed too.</p>
				<Button kind="destructive" label="Delete plan" onclick={deletePlan} />
				<Button kind="plain" label="Keep it" onclick={() => (editor.confirmDelete = false)} />
			{:else}
				<Button kind="ghost" label="Delete this plan" onclick={() => (editor.confirmDelete = true)} />
			{/if}
		{/if}
	</div>
</div>

<style>
	.crumb {
		display: flex;
		align-items: center;
		gap: 6px;
		height: 16px;
		color: var(--ink-3);
		font: 500 13px/16px var(--font-ui);
	}

	.crumb button {
		color: var(--accent);
		font: inherit;
	}

	.crumb span {
		color: var(--ink-2);
	}

	.form {
		max-width: 640px;
		margin-top: 20px;
	}

	.named {
		display: flex;
		flex-direction: column;
		gap: 6px;
		margin-bottom: 16px;
	}

	.label {
		color: var(--ink);
		font: 600 13px/16px var(--font-ui);
	}

	.pace {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 10px;
		margin-bottom: 24px;
	}

	.pace :global(.in-days) {
		width: 72px;
	}

	.section {
		margin: 8px 0 8px;
		color: var(--ink);
		font: 600 13px/20px var(--font-ui);
	}

	.hint {
		margin: 0 0 10px;
		color: var(--ink-2);
		font: 400 13px/20px var(--font-ui);
	}

	.days-head {
		margin: 24px 0 12px;
		color: var(--ink);
		font: 600 13px/20px var(--font-ui);
	}

	.days {
		display: flex;
		flex-direction: column;
		gap: 8px;
		margin: 0 0 12px;
		padding: 0;
		list-style: none;
	}

	.day {
		display: flex;
		align-items: center;
		gap: 8px;
	}

	.num {
		width: 20px;
		color: var(--ink-3);
		font: 500 12px/16px var(--font-ui);
		text-align: right;
	}

	.day :global(.field) {
		flex: 1;
	}

	.error {
		margin: 16px 0 0;
		color: var(--danger);
		font: 500 13px/20px var(--font-ui);
	}

	.warn {
		flex-basis: 100%;
		margin: 8px 0 0;
		color: var(--ink-2);
		font: 400 13px/20px var(--font-ui);
	}

	.actions {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 8px;
		margin-top: 20px;
	}
</style>
