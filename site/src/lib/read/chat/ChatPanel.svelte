<!--
	The Bible chat panel beside the reader: a conversation about whatever it was opened on (verses,
	a chapter, a book, notes, or anything), with replies streaming in. Its header starts a new
	conversation, shows past ones, widens the panel and opens its settings; without an API key the
	panel asks for one first, and sends the waiting question once it's saved.
-->
<script lang="ts">
	import { tick } from 'svelte';
	import { getApp } from '../app.svelte';
	import { Icon, Menu, MenuItem, MenuSeparator, RoundButton } from '../ui';
	import ChatComposer from './ChatComposer.svelte';
	import ChatHistory from './ChatHistory.svelte';
	import ChatReply from './ChatReply.svelte';
	import ChatSettings from './ChatSettings.svelte';
	import KeySetup from './KeySetup.svelte';
	import { contextIcon, contextKind, contextLabel, contextPlace, suggestions, type ChatContext } from './context';

	interface Props {
		top: number;
		width: number;
		/** Covers the window (a phone). */
		cover: boolean;
	}

	let { top, width, cover }: Props = $props();

	const app = getApp();
	const chat = app.chat;

	let scroller = $state<HTMLElement>();
	let newButton = $state<HTMLButtonElement>();
	let newMenu = $state(false);
	/** Following the reply as it streams, until the reader scrolls up. */
	let pinned = true;

	const conv = $derived(chat.active);
	const context = $derived(chat.context);
	const translation = $derived(conv?.translation ?? app.settings.translation);
	const abbrev = $derived(app.info(translation)?.abbrev ?? '');
	const modelName = $derived(chat.provider.models.find((m) => m.id === chat.config.model)?.label ?? chat.config.model);
	const view = $derived(chat.view);
	const needsKey = $derived(!chat.hasKey && view === 'chat');

	const heading = $derived(view === 'history' ? 'History' : view === 'settings' ? 'Chat settings' : conv ? conv.title : 'Bible Chat');
	const sub = $derived(
		view === 'history'
			? 'Saved in this browser'
			: view === 'settings'
				? `${chat.provider.name} · your own key`
				: `${contextLabel(app, context)} · ${abbrev}${chat.hasKey ? ` · ${modelName}` : ''}`
	);

	const placeholder = $derived.by(() => {
		if (conv?.messages.length) return 'Ask a follow-up…';
		switch (context.kind) {
			case 'general':
				return 'Ask anything, or share what you’re facing…';
			case 'notes':
				return 'Ask about your notes…';
			default:
				return `Ask about ${contextLabel(app, context)}…`;
		}
	});

	/** Contexts a new conversation can be about, from where the reader is. */
	const starts = $derived.by(() => {
		const out: { context: ChatContext; label: string; detail: string }[] = [];
		const refs = app.selectedRefs();
		if (refs.length) {
			const c: ChatContext = { kind: 'verses', from: refs[0], to: refs[refs.length - 1] };
			out.push({ context: c, label: contextLabel(app, c), detail: 'Selected' });
		}
		const chapter: ChatContext = { kind: 'chapter', chapter: app.here.chapter };
		out.push({ context: chapter, label: contextLabel(app, chapter), detail: 'Chapter' });
		const book: ChatContext = { kind: 'book', book: app.hereRef().book };
		out.push({ context: book, label: `The book of ${contextLabel(app, book)}`, detail: 'Book' });
		if (app.counts().notes > 0) out.push({ context: { kind: 'notes' }, label: 'My notes', detail: `${app.counts().notes}` });
		return out;
	});

	/** The verses a passage conversation is about, quoted. */
	const quote = $derived.by(() => {
		if (context.kind !== 'verses' && context.kind !== 'note') return '';
		const [from, to] = context.kind === 'verses' ? [context.from, context.to] : [context.verse, context.verse];
		const parts: string[] = [];
		for (let v = from.verse; v <= to.verse && parts.length < 6; v++) {
			const t = app.loaded.has(translation) ? app.core?.verse(translation, { ...from, verse: v }) : null;
			if (t) parts.push(t.runs.map((r) => r.text).join('').trim());
		}
		return parts.join(' ');
	});

	function startNew(c: ChatContext) {
		newMenu = false;
		chat.newChat(c);
	}

	function ask(q: string) {
		void chat.send(q);
	}

	function goTo(c: ChatContext) {
		const p = contextPlace(app, c);
		if (!p) return;
		app.dispatch({ type: 'jump', chapter: p.chapter, verse: p.verse, flash: p.verse !== undefined });
		if (cover) chat.close();
	}

	async function copy(text: string) {
		try {
			await navigator.clipboard.writeText(text.replace(/\[\[([^\]]+)\]\]/g, '$1'));
			app.toastNow('Copied');
		} catch {
			app.toastNow("Couldn't copy");
		}
	}

	function onscroll() {
		if (!scroller) return;
		pinned = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 48;
	}

	// Follows the reply as it grows, and opens a conversation at its end.
	$effect(() => {
		const last = conv?.messages.at(-1);
		void last?.content.length;
		void last?.steps?.length;
		void chat.streaming;
		void conv?.id;
		if (pinned) void tick().then(() => scroller?.scrollTo({ top: scroller.scrollHeight }));
	});

	$effect(() => {
		void conv?.id;
		pinned = true;
	});

	// Development: `chat-scroll:top` shows a conversation from its start.
	$effect(() => {
		if (app.demo['chat-scroll'] === 'top' && scroller) {
			pinned = false;
			void tick().then(() => setTimeout(() => scroller?.scrollTo({ top: 0 }), 50));
		}
	});

	// A question sent pins the view to the bottom again.
	$effect(() => {
		void conv?.messages.length;
		pinned = true;
	});
</script>

<aside
	class={['chat', { cover }]}
	style:top={cover ? undefined : `${top}px`}
	style:width={cover ? undefined : `${width - 8}px`}
	aria-label="Bible chat"
>
	<header class="head">
		{#if view !== 'chat'}
			<RoundButton icon="chevron-left" label="Back to the conversation" onclick={() => (chat.view = 'chat')} />
		{:else}
			<span class="mark"><Icon name="sparkle" size={16} heavy /></span>
		{/if}
		<div class="titles">
			<h2 class="title">{heading}</h2>
			<p class="detail">{sub}</p>
		</div>
		<RoundButton icon="plus" label="New conversation" bind:el={newButton} onclick={() => (newMenu = !newMenu)} />
		<RoundButton
			icon="history"
			label="History"
			on={view === 'history'}
			onclick={() => (chat.view = view === 'history' ? 'chat' : 'history')}
		/>
		{#if !cover}
			<RoundButton
				icon={chat.expanded ? 'contract' : 'expand'}
				label={chat.expanded ? 'Make narrower' : 'Make wider'}
				onclick={() => (chat.expanded = !chat.expanded)}
			/>
		{/if}
		<RoundButton
			icon="gear"
			label="Chat settings"
			on={view === 'settings'}
			onclick={() => (chat.view = view === 'settings' ? 'chat' : 'settings')}
		/>
		<RoundButton icon="close" label="Close chat" onclick={() => chat.close()} />
	</header>

	{#if view === 'history'}
		<div class="scroll"><ChatHistory /></div>
	{:else if view === 'settings'}
		<div class="scroll"><ChatSettings /></div>
	{:else}
		<div class="scroll" bind:this={scroller} {onscroll}>
			<div class="thread" class:wide={chat.expanded && !cover}>
				{#if context.kind !== 'general'}
					<div class="context">
						<div class="context-head">
							<span class="context-glyph"><Icon name={contextIcon(context)} size={14} heavy /></span>
							<span class="context-label">{contextLabel(app, context)}</span>
							<span class="context-kind">{contextKind(context)} · {abbrev}</span>
							{#if contextPlace(app, context)}
								<button type="button" class="context-open" onclick={() => goTo(context)}>Open<Icon name="arrow-right" size={12} heavy /></button>
							{/if}
						</div>
						{#if quote}<p class="context-quote">{quote}</p>{/if}
					</div>
				{/if}

				{#if !conv?.messages.length && !chat.pending}
					<div class="intro">
						{#if context.kind === 'general'}
							<span class="intro-glyph"><Icon name="sparkle" size={22} heavy /></span>
							<h3 class="intro-title">Ask anything about the Bible</h3>
							<p class="intro-body">
								Tell me what you’re going through and I’ll find verses to hold on to, or ask about any passage. References
								in answers open in the reader.
							</p>
						{:else}
							<p class="intro-lead">Suggested questions</p>
						{/if}
						<div class="suggestions">
							{#each suggestions(context) as s (s)}
								<button type="button" class="suggestion" onclick={() => ask(s)}>
									<span>{s}</span>
									<Icon name="arrow-up" size={14} heavy />
								</button>
							{/each}
						</div>
						{#if context.kind === 'general'}
							<p class="intro-lead">Or talk about</p>
							<div class="starts">
								{#each starts as s (s.label)}
									<button type="button" class="start" onclick={() => (chat.draft = s.context)}>
										<Icon name={contextIcon(s.context)} size={12} heavy />{s.label}
									</button>
								{/each}
							</div>
						{/if}
					</div>
				{/if}

				{#each conv?.messages ?? [] as m, i (m.id)}
					{#if m.role === 'user'}
						<div class="user"><p>{m.content}</p></div>
					{:else}
						{@const live = chat.streaming === conv?.id && i === (conv?.messages.length ?? 0) - 1}
						<div class="assistant">
							<ChatReply message={m} translation={translation} {context} streaming={live} onfollow={() => cover && chat.close()} />
							{#if m.error}
								<div class="error" role="alert">
									<Icon name="warning" size={16} heavy />
									<div class="error-text">
										<p>{m.error.message}</p>
										<div class="error-buttons">
											<button type="button" onclick={() => chat.retry()}>Try again</button>
											{#if m.error.kind === 'auth' || m.error.kind === 'model'}
												<button type="button" onclick={() => (chat.view = 'settings')}>
													{m.error.kind === 'auth' ? 'Update key' : 'Choose model'}
												</button>
											{/if}
										</div>
									</div>
								</div>
							{:else if !live && m.content && i === (conv?.messages.length ?? 0) - 1}
								<div class="actions">
									<button type="button" title="Copy answer" aria-label="Copy answer" onclick={() => copy(m.content)}>
										<Icon name="copy" size={14} heavy />
									</button>
									<button type="button" title="Ask again" aria-label="Ask again" onclick={() => chat.retry()}>
										<Icon name="retry" size={14} heavy />
									</button>
								</div>
							{/if}
						</div>
					{/if}
				{/each}

				{#if needsKey}
					{#if chat.pending}
						<div class="user waiting">
							<p>{chat.pending}</p>
							<span class="waiting-note">Sends once your key is saved</span>
						</div>
					{/if}
					<KeySetup />
				{/if}
			</div>
		</div>
		{#if !needsKey}
			<ChatComposer {context} {placeholder} wide={chat.expanded && !cover} />
		{/if}
	{/if}
</aside>

{#if newMenu && newButton}
	<Menu anchor={newButton} align="end" width={250} label="New conversation" onclose={() => (newMenu = false)}>
		<MenuItem icon="sparkle" label="Anything · find verses" onclick={() => startNew({ kind: 'general' })} />
		<MenuSeparator />
		{#each starts as s (s.label)}
			<MenuItem icon={contextIcon(s.context)} label={s.label} shortcut={s.detail} onclick={() => startNew(s.context)} />
		{/each}
	</Menu>
{/if}

<style>
	.chat {
		position: absolute;
		right: 8px;
		bottom: 8px;
		display: flex;
		flex-direction: column;
		border-radius: var(--radius-lg);
		background: var(--sidebar);
		box-shadow: var(--shadow-panel);
		overflow: hidden;
		z-index: 5;
		animation: slide var(--motion-base) var(--motion-easing);
	}

	.chat.cover {
		position: fixed;
		inset: 0;
		border-radius: 0;
		z-index: 60;
	}

	@keyframes slide {
		from {
			opacity: 0;
			transform: translateX(12px);
		}
	}

	.head {
		display: flex;
		align-items: center;
		gap: 2px;
		flex-shrink: 0;
		height: 60px;
		padding: 0 10px 0 14px;
		box-shadow: inset 0 -1px 0 var(--separator);
	}

	.mark {
		display: grid;
		flex-shrink: 0;
		place-items: center;
		width: 28px;
		height: 28px;
		margin-right: 4px;
		border-radius: 8px;
		background: var(--accent);
		color: var(--on-accent);
	}

	.titles {
		flex: 1;
		min-width: 0;
		margin-left: 4px;
	}

	.title {
		margin: 0;
		overflow: hidden;
		color: var(--ink);
		font: var(--type-ui-title);
		letter-spacing: var(--type-ui-title-tracking);
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.detail {
		margin: 1px 0 0;
		overflow: hidden;
		color: var(--ink-2);
		font: var(--type-small);
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.scroll {
		flex: 1;
		min-height: 0;
		overflow-y: auto;
		overscroll-behavior: contain;
	}

	.thread {
		display: flex;
		flex-direction: column;
		gap: 16px;
		padding: 16px 16px 20px;
	}

	.thread.wide {
		max-width: 720px;
		margin: 0 auto;
		padding: 20px 24px 24px;
	}

	.context {
		padding: 10px 12px;
		border-radius: 12px;
		background: var(--surface);
		box-shadow: inset 0 0 0 1px var(--separator);
	}

	.context-head {
		display: flex;
		align-items: center;
		gap: 6px;
		min-width: 0;
	}

	.context-glyph {
		display: flex;
		color: var(--accent);
	}

	.context-label {
		overflow: hidden;
		color: var(--ink);
		font: var(--type-ui-strong);
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.context-kind {
		flex: 1;
		overflow: hidden;
		color: var(--ink-3);
		font: 400 11px/16px var(--font-ui);
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.context-open {
		display: inline-flex;
		align-items: center;
		gap: 3px;
		padding: 2px 6px;
		border-radius: 8px;
		color: var(--accent);
		font: 500 12px/16px var(--font-ui);
	}

	.context-open:hover {
		background: var(--fill);
	}

	.context-quote {
		display: -webkit-box;
		margin: 8px 0 2px;
		padding-left: 10px;
		overflow: hidden;
		-webkit-box-orient: vertical;
		-webkit-line-clamp: 4;
		line-clamp: 4;
		box-shadow: inset 2px 0 var(--accent);
		color: var(--ink-2);
		font: italic 400 14px/21px var(--font-reading);
	}

	.intro {
		display: flex;
		flex-direction: column;
		gap: 8px;
	}

	.intro-glyph {
		display: grid;
		place-items: center;
		width: 44px;
		height: 44px;
		margin: 12px 0 4px;
		border-radius: 13px;
		background: var(--accent-soft);
		color: var(--accent);
	}

	.intro-title {
		margin: 0;
		color: var(--ink);
		font: 600 18px/24px var(--font-ui);
		letter-spacing: -0.01em;
	}

	.intro-body {
		margin: 0 0 8px;
		color: var(--ink-2);
		font: 400 13px/19px var(--font-ui);
	}

	.intro-lead {
		margin: 4px 0 0;
		color: var(--ink-3);
		font: 600 11px/16px var(--font-ui);
		letter-spacing: 0.04em;
		text-transform: uppercase;
	}

	.suggestions {
		display: flex;
		flex-direction: column;
		gap: 6px;
	}

	.suggestion {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 10px;
		padding: 9px 12px;
		border-radius: 12px;
		background: var(--surface);
		box-shadow: inset 0 0 0 1px var(--separator);
		color: var(--ink);
		font: 400 13px/18px var(--font-ui);
		text-align: left;
	}

	.suggestion :global(svg) {
		flex-shrink: 0;
		color: var(--ink-3);
		transform: rotate(45deg);
	}

	.suggestion:hover {
		background-image: linear-gradient(var(--fill), var(--fill));
	}

	.suggestion:hover :global(svg) {
		color: var(--accent);
	}

	.starts {
		display: flex;
		flex-wrap: wrap;
		gap: 6px;
	}

	.start {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		height: 28px;
		padding: 0 11px;
		border-radius: 14px;
		background: var(--raised);
		box-shadow: var(--shadow-control);
		color: var(--ink);
		font: 500 12px/16px var(--font-ui);
	}

	.start :global(svg) {
		color: var(--accent);
	}

	.start:hover {
		background-image: linear-gradient(var(--fill), var(--fill));
	}

	.user {
		display: flex;
		flex-direction: column;
		align-items: flex-end;
		margin-left: 40px;
	}

	.user p {
		margin: 0;
		padding: 8px 12px;
		border-radius: 16px 16px 4px 16px;
		background: var(--accent);
		color: var(--on-accent);
		font: 400 14px/20px var(--font-ui);
		white-space: pre-wrap;
		overflow-wrap: anywhere;
	}

	.user.waiting p {
		opacity: 0.6;
	}

	.waiting-note {
		margin-top: 4px;
		color: var(--ink-3);
		font: 400 11px/14px var(--font-ui);
	}

	.assistant {
		display: flex;
		flex-direction: column;
		gap: 8px;
	}

	.actions {
		display: flex;
		gap: 2px;
		margin-left: -6px;
	}

	.actions button {
		display: grid;
		place-items: center;
		width: 26px;
		height: 26px;
		border-radius: 50%;
		color: var(--ink-3);
	}

	.actions button:hover {
		background: var(--fill);
		color: var(--ink);
	}

	.error {
		display: flex;
		gap: 8px;
		padding: 10px 12px;
		border-radius: 12px;
		background: color-mix(in srgb, var(--danger) 9%, transparent);
		color: var(--danger);
	}

	.error-text p {
		margin: 0;
		color: var(--ink);
		font: 400 13px/18px var(--font-ui);
	}

	.error-buttons {
		display: flex;
		gap: 12px;
		margin-top: 6px;
	}

	.error-buttons button {
		color: var(--accent);
		font: 600 12px/16px var(--font-ui);
	}

	.error-buttons button:hover {
		text-decoration: underline;
	}
</style>
