/**
 * The Bible chat: conversations with a language model about a passage, a chapter, a book, notes
 * or anything at all, on the reader's own API key. Conversations and the key are kept in this
 * browser (localStorage) and the requests go straight to the provider.
 *
 * Any part of the reader starts a conversation with `start(context, question?)` or offers the
 * Ask sheet with `askAbout(context)`; the panel (ChatPanel.svelte) shows whatever is active.
 */
import type { ReadApp } from '../app.svelte';
import { contextLabel, contextText, type ChatContext } from './context';
import { PROVIDERS } from './openai';
import { ProviderError, type ErrorKind, type Provider, type ToolCall, type WireMessage } from './provider';
import { runTool, toolPending, TOOLS } from './tools';

export const CHATS_KEY = 'bible.read.chats';
export const ASSISTANT_KEY = 'bible.read.assistant';
const KEPT = 60;
const TOOL_ROUNDS = 4;
const SAVE_MS = 600;

export interface Step {
	label: string;
	done: boolean;
}

export interface ChatMessage {
	id: string;
	role: 'user' | 'assistant';
	content: string;
	/** Tools the model used for this reply, as the reader sees them. */
	steps?: Step[];
	error?: { kind: ErrorKind; message: string };
	/** Stopped by the reader before it finished. */
	stopped?: boolean;
	model?: string;
	at: number;
}

export interface Conversation {
	id: string;
	title: string;
	context: ChatContext;
	/** The translation the context and quotes are in. */
	translation: string;
	messages: ChatMessage[];
	created: number;
	updated: number;
}

export interface AssistantConfig {
	provider: string;
	key: string;
	model: string;
}

export type ChatView = 'chat' | 'history' | 'settings';

const uid = () => Math.random().toString(36).slice(2, 10) + Date.now().toString(36);
const nowMs = () => Date.now();

function load<T>(key: string, fallback: T): T {
	try {
		const raw = localStorage.getItem(key);
		return raw ? (JSON.parse(raw) as T) : fallback;
	} catch {
		return fallback;
	}
}

export class Chat {
	/** The panel beside the reader. */
	open = $state(false);
	/** Wider, for reading long answers. */
	expanded = $state(false);
	view = $state<ChatView>('chat');
	conversations = $state<Conversation[]>([]);
	activeId = $state<string | null>(null);
	/** What a new conversation will be about, until its first question. */
	draft = $state.raw<ChatContext>({ kind: 'general' });
	/** The Ask sheet, open on a context. */
	asking = $state.raw<ChatContext | null>(null);
	/** The conversation a reply is streaming into. */
	streaming = $state<string | null>(null);
	config = $state<AssistantConfig>({ provider: 'openai', key: '', model: PROVIDERS[0].defaultModel });
	/** A question asked before there was a key: sent once one is saved. */
	pending = $state<string | null>(null);
	/** Bumped to put the cursor in the composer. */
	focusRequest = $state(0);
	/** Development only: a stand-in for the provider (demo.ts). */
	stub: Provider | null = null;

	#abort: AbortController | null = null;
	#save: ReturnType<typeof setTimeout> | undefined;
	#loaded = false;

	constructor(private app: ReadApp) {}

	get provider(): Provider {
		return this.stub ?? PROVIDERS.find((p) => p.id === this.config.provider) ?? PROVIDERS[0];
	}

	get hasKey(): boolean {
		return !!this.config.key.trim();
	}

	get active(): Conversation | null {
		return this.conversations.find((c) => c.id === this.activeId) ?? null;
	}

	/** The context shown: the active conversation's, or the draft's. */
	get context(): ChatContext {
		return this.active?.context ?? this.draft;
	}

	/** Reads what this browser kept (once, after hydration). */
	restore() {
		if (this.#loaded) return;
		this.#loaded = true;
		const saved = load<Partial<AssistantConfig>>(ASSISTANT_KEY, {});
		this.config = {
			provider: typeof saved.provider === 'string' ? saved.provider : 'openai',
			key: typeof saved.key === 'string' ? saved.key : '',
			model: typeof saved.model === 'string' && saved.model ? saved.model : this.provider.defaultModel
		};
		const list = load<Conversation[]>(CHATS_KEY, []);
		this.conversations = Array.isArray(list) ? list.filter((c) => c && Array.isArray(c.messages)) : [];
	}

	/** Another tab saved its conversations. */
	storageChanged(key: string | null) {
		if (key === CHATS_KEY && !this.streaming) {
			const list = load<Conversation[]>(CHATS_KEY, []);
			if (Array.isArray(list)) this.conversations = list;
		} else if (key === ASSISTANT_KEY) {
			this.#loaded = false;
			const convs = this.conversations;
			this.restore();
			this.conversations = convs;
		}
	}

	// ------------------------------------------------------------ the panel

	toggle() {
		if (this.open) this.close();
		else this.show();
	}

	show() {
		this.restore();
		this.open = true;
		this.app.panel = null;
		if (this.view === 'chat') this.focusRequest++;
	}

	close() {
		this.open = false;
	}

	/** A blank conversation about `context`, in the panel. */
	newChat(context: ChatContext = { kind: 'general' }) {
		this.activeId = null;
		this.draft = context;
		this.view = 'chat';
		this.show();
	}

	select(id: string) {
		this.activeId = id;
		this.view = 'chat';
		this.show();
	}

	/** The Ask sheet for a context (with no key, the panel asks for one first). */
	askAbout(context: ChatContext) {
		this.restore();
		this.asking = context;
	}

	/** A new conversation about `context`, asking `question` if there is one. */
	start(context: ChatContext, question?: string) {
		this.asking = null;
		this.newChat(context);
		if (question?.trim()) void this.send(question);
	}

	// ------------------------------------------------------------ the key

	async saveKey(key: string, model?: string): Promise<string | null> {
		const k = key.trim();
		if (!k) return 'Paste a key first.';
		try {
			await this.provider.verify(k);
		} catch (e) {
			if (e instanceof ProviderError && e.kind !== 'network') return e.message;
			// Offline or blocked: keep the key; the first question will say if it's wrong.
		}
		this.config.key = k;
		if (model) this.config.model = model;
		this.saveConfig();
		const q = this.pending;
		this.pending = null;
		this.view = 'chat';
		if (q) void this.send(q);
		return null;
	}

	removeKey() {
		this.config.key = '';
		this.saveConfig();
	}

	setModel(model: string) {
		this.config.model = model.trim() || this.provider.defaultModel;
		this.saveConfig();
	}

	saveConfig() {
		try {
			localStorage.setItem(ASSISTANT_KEY, JSON.stringify($state.snapshot(this.config)));
		} catch {
			this.app.toastNow("Couldn't save chat settings in this browser");
		}
	}

	// ------------------------------------------------------------ history

	remove(id: string) {
		if (this.streaming === id) this.stop();
		this.conversations = this.conversations.filter((c) => c.id !== id);
		if (this.activeId === id) this.activeId = null;
		this.saveSoon(0);
	}

	clearHistory() {
		this.stop();
		this.conversations = [];
		this.activeId = null;
		this.saveSoon(0);
	}

	saveSoon(ms = SAVE_MS) {
		clearTimeout(this.#save);
		this.#save = setTimeout(() => this.saveNow(), ms);
	}

	saveNow() {
		clearTimeout(this.#save);
		let list = $state.snapshot(this.conversations)
			.sort((a, b) => b.updated - a.updated)
			.slice(0, KEPT);
		// Full storage: drop the oldest until it fits.
		while (list.length) {
			try {
				localStorage.setItem(CHATS_KEY, JSON.stringify(list));
				return;
			} catch {
				list = list.slice(0, Math.floor(list.length * 0.75));
			}
		}
		try {
			localStorage.removeItem(CHATS_KEY);
		} catch {
			// Blocked storage: conversations last for the visit.
		}
	}

	// ------------------------------------------------------------ asking

	stop() {
		this.#abort?.abort();
	}

	/** Asks a question in the active conversation (or a new one about the draft context). */
	async send(text: string) {
		const question = text.trim();
		if (!question || this.streaming) return;
		this.restore();
		if (!this.hasKey) {
			this.pending = question;
			this.show();
			return;
		}
		let conv = this.active;
		if (!conv) {
			conv = {
				id: uid(),
				title: title(question),
				context: this.draft,
				translation: this.app.settings.translation,
				messages: [],
				created: nowMs(),
				updated: nowMs()
			};
			this.conversations = [conv, ...this.conversations];
			this.activeId = conv.id;
			conv = this.active!;
		}
		conv.messages.push({ id: uid(), role: 'user', content: question, at: nowMs() });
		await this.#reply(conv.id);
	}

	/** Asks the last question again (after an error, or to get another answer). */
	async retry() {
		const conv = this.active;
		if (!conv || this.streaming) return;
		const last = conv.messages.at(-1);
		if (last?.role === 'assistant') conv.messages.pop();
		if (conv.messages.at(-1)?.role === 'user') await this.#reply(conv.id);
	}

	async #reply(id: string) {
		const find = () => this.conversations.find((c) => c.id === id);
		const conv = find();
		if (!conv) return;
		const provider = this.provider;
		const model = this.config.model || provider.defaultModel;
		const abort = new AbortController();
		this.#abort = abort;
		this.streaming = id;
		conv.messages.push({ id: uid(), role: 'assistant', content: '', steps: [], model, at: nowMs() });
		const msg = () => find()?.messages.at(-1);

		try {
			const context = await contextText(this.app, conv.context, conv.translation);
			const wire: WireMessage[] = [
				{ role: 'system', content: systemPrompt(this.app, conv, context) },
				...conv.messages
					.slice(0, -1)
					.filter((m) => !m.error && (m.role === 'user' || m.content))
					.map((m): WireMessage => ({ role: m.role, content: m.content }))
			];
			for (let round = 0; round <= TOOL_ROUNDS; round++) {
				let text = '';
				let calls: ToolCall[] = [];
				// The last round has no tools, so it must answer.
				const tools = round < TOOL_ROUNDS ? TOOLS : [];
				for await (const ev of provider.stream({ key: this.config.key, model, messages: wire, tools, signal: abort.signal })) {
					const m = msg();
					if (!m) break;
					if (ev.type === 'text') {
						text += ev.text;
						m.content += ev.text;
					} else calls = ev.calls;
				}
				if (!calls.length) break;
				wire.push({ role: 'assistant', content: text || null, toolCalls: calls });
				for (const call of calls) {
					const step: Step = { label: toolPending(call.name, call.args), done: false };
					msg()?.steps?.push(step);
					const at = (msg()?.steps?.length ?? 1) - 1;
					const result = await runTool(this.app, conv.translation, call.name, call.args);
					const s = msg()?.steps?.[at];
					if (s) {
						s.label = result.label;
						s.done = true;
					}
					wire.push({ role: 'tool', toolCallId: call.id, content: result.content });
				}
				if (abort.signal.aborted) break;
				// Text before the tools ran reads as its own paragraph.
				const m = msg();
				if (m?.content && !m.content.endsWith('\n')) m.content += '\n\n';
			}
			if (abort.signal.aborted) {
				const m = msg();
				if (m) m.stopped = true;
			}
		} catch (e) {
			const m = msg();
			if (m) {
				if (abort.signal.aborted || (e instanceof DOMException && e.name === 'AbortError')) m.stopped = true;
				else
					m.error =
						e instanceof ProviderError
							? { kind: e.kind, message: e.message }
							: { kind: 'other', message: e instanceof Error ? e.message : String(e) };
			}
		} finally {
			const c = find();
			if (c) c.updated = nowMs();
			if (this.#abort === abort) this.#abort = null;
			this.streaming = null;
			this.saveSoon(0);
		}
	}
}

/** A conversation's title: its first question, cut at a word. */
function title(q: string): string {
	const one = q.replace(/\s+/g, ' ').trim();
	if (one.length <= 60) return one;
	const cut = one.slice(0, 60);
	return `${cut.slice(0, Math.max(30, cut.lastIndexOf(' ')))}…`;
}

function systemPrompt(app: ReadApp, conv: Conversation, context: string): string {
	const info = app.info(conv.translation);
	const translation = info ? `${info.name} (${info.abbrev})` : conv.translation.toUpperCase();
	const about =
		conv.context.kind === 'general'
			? 'The reader has not picked a passage: help them find verses and answer questions about the Bible as a whole.'
			: `This conversation is about ${contextLabel(app, conv.context)}.`;
	return `You are a warm, wise and accurate Bible study companion inside a Bible reading app. The reader is reading the ${translation}.

${about}

How to answer:
- Write every Bible reference in double square brackets with the full book name: [[John 3:16]], [[Romans 8:28-30]], [[Psalm 23]]. The app turns them into links and shows the text of each cited passage from the reader's translation beneath your reply, so don't paste whole verses; quote only the words you are discussing, from the ${info?.abbrev ?? 'reader’s translation'}.
- When the reader asks for verses for a need (motivation, anxiety, grief, hard times, a decision), offer 3 to 5 well-chosen passages, each with a sentence or two on why it speaks to them. Use search_bible or read_passage to find or check passages when you aren't sure of the exact wording or reference. Never invent a reference.
- Be faithful to the text: say what it says, then interpretation. Give historical and literary context when it helps. Where Christian traditions read a passage differently, say so briefly and fairly.
- Be gentle and pastoral with someone who is struggling. If they mention wanting to harm themselves or being in danger, encourage them to reach out now to someone they trust or a crisis line (such as 988 in the US) alongside any Scripture.
- Keep replies focused and readable: short paragraphs, **bold** for key ideas, lists where they help. No headings for short answers.${
		context ? `\n\n---\n${context}` : ''
	}`;
}
