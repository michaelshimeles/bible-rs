/**
 * OpenAI's Chat Completions API, streamed, with tools. OpenAI allows calls from the browser, so
 * the reader's key goes from this page to api.openai.com and nowhere else.
 */
import { ProviderError, sseData, type Provider, type StreamEvent, type StreamRequest, type ToolCall, type WireMessage } from './provider';

const API = 'https://api.openai.com/v1';

/** Reasoning models take `reasoning_effort` and no sampling settings. */
const reasoning = (model: string) => /^(o\d|gpt-5)/.test(model);

function wire(m: WireMessage): Record<string, unknown> {
	switch (m.role) {
		case 'assistant':
			return {
				role: 'assistant',
				content: m.content,
				...(m.toolCalls?.length
					? {
							tool_calls: m.toolCalls.map((c) => ({
								id: c.id,
								type: 'function',
								function: { name: c.name, arguments: c.args }
							}))
						}
					: {})
			};
		case 'tool':
			return { role: 'tool', tool_call_id: m.toolCallId, content: m.content };
		default:
			return { role: m.role, content: m.content };
	}
}

/** A failed response as a ProviderError, with OpenAI's own message where it gave one. */
async function failure(res: Response): Promise<ProviderError> {
	let message = '';
	let code = '';
	try {
		const body = await res.json();
		message = body?.error?.message ?? '';
		code = body?.error?.code ?? '';
	} catch {
		// Not JSON.
	}
	if (res.status === 401) return new ProviderError('auth', "OpenAI didn't accept this API key. Check it, or make a new one.");
	if (res.status === 429 || code === 'insufficient_quota')
		return new ProviderError(
			'quota',
			code === 'insufficient_quota'
				? 'Your OpenAI account is out of credit. Add some in the OpenAI billing settings.'
				: 'OpenAI is rate limiting this key. Wait a moment and try again.'
		);
	if (res.status === 404 || code === 'model_not_found')
		return new ProviderError('model', message || "This key can't use that model. Pick another in chat settings.");
	return new ProviderError('other', message || `OpenAI returned an error (${res.status}).`);
}

async function post(url: string, init: RequestInit): Promise<Response> {
	try {
		return await fetch(url, init);
	} catch (e) {
		if (init.signal?.aborted) throw e;
		const offline = typeof navigator !== 'undefined' && !navigator.onLine;
		throw new ProviderError('network', offline ? "You're offline." : "Couldn't reach OpenAI. Check your connection.");
	}
}

export const openai: Provider = {
	id: 'openai',
	name: 'OpenAI',
	models: [
		{ id: 'gpt-4.1-mini', label: 'GPT-4.1 mini', detail: 'Fast, thoughtful answers' },
		{ id: 'gpt-4.1-nano', label: 'GPT-4.1 nano', detail: 'Fastest and cheapest' },
		{ id: 'gpt-4.1', label: 'GPT-4.1', detail: 'More thorough, slower' },
		{ id: 'gpt-5-mini', label: 'GPT-5 mini', detail: 'Thinks before answering' }
	],
	defaultModel: 'gpt-4.1-mini',
	keyUrl: 'https://platform.openai.com/api-keys',
	keyPlaceholder: 'sk-…',

	async verify(key, signal) {
		const res = await post(`${API}/models?limit=1`, { headers: { Authorization: `Bearer ${key}` }, signal });
		if (!res.ok) throw await failure(res);
	},

	async *stream({ key, model, messages, tools, signal }: StreamRequest): AsyncGenerator<StreamEvent> {
		const res = await post(`${API}/chat/completions`, {
			method: 'POST',
			headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${key}` },
			signal,
			body: JSON.stringify({
				model,
				stream: true,
				messages: messages.map(wire),
				...(tools.length
					? {
							tools: tools.map((t) => ({
								type: 'function',
								function: { name: t.name, description: t.description, parameters: t.parameters }
							}))
						}
					: {}),
				...(reasoning(model) ? { reasoning_effort: 'low' } : { temperature: 0.5 })
			})
		});
		if (!res.ok || !res.body) throw await failure(res);

		const calls: ToolCall[] = [];
		for await (const data of sseData(res.body, signal)) {
			if (data === '[DONE]') break;
			let chunk;
			try {
				chunk = JSON.parse(data);
			} catch {
				continue;
			}
			if (chunk.error) throw new ProviderError('other', chunk.error.message ?? 'OpenAI stopped with an error.');
			const delta = chunk.choices?.[0]?.delta;
			if (!delta) continue;
			if (typeof delta.content === 'string' && delta.content) yield { type: 'text', text: delta.content };
			for (const t of delta.tool_calls ?? []) {
				const i: number = t.index ?? calls.length;
				calls[i] ??= { id: '', name: '', args: '' };
				if (t.id) calls[i].id = t.id;
				if (t.function?.name) calls[i].name += t.function.name;
				if (t.function?.arguments) calls[i].args += t.function.arguments;
			}
		}
		const asked = calls.filter((c) => c && c.name);
		if (asked.length) yield { type: 'toolCalls', calls: asked };
	}
};

export const PROVIDERS: Provider[] = [openai];
