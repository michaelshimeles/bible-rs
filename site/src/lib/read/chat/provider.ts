/**
 * What the chat needs of a language model: a streamed reply that may ask for tools. Each
 * provider (OpenAI today) turns this into its own API. Requests go straight from the browser to
 * the provider with the reader's own key; nothing passes through this site.
 */

export interface ToolCall {
	id: string;
	name: string;
	/** JSON, as the model wrote it. */
	args: string;
}

export type WireMessage =
	| { role: 'system' | 'user'; content: string }
	| { role: 'assistant'; content: string | null; toolCalls?: ToolCall[] }
	| { role: 'tool'; toolCallId: string; content: string };

export interface ToolSpec {
	name: string;
	description: string;
	/** JSON Schema for the arguments. */
	parameters: Record<string, unknown>;
}

export type StreamEvent = { type: 'text'; text: string } | { type: 'toolCalls'; calls: ToolCall[] };

export interface StreamRequest {
	key: string;
	model: string;
	messages: WireMessage[];
	tools: ToolSpec[];
	signal: AbortSignal;
}

export interface ModelOption {
	id: string;
	label: string;
	detail: string;
}

export interface Provider {
	id: string;
	name: string;
	models: ModelOption[];
	defaultModel: string;
	/** Where to make a key. */
	keyUrl: string;
	keyPlaceholder: string;
	/** Text as it arrives, then the tools asked for (if any) once the reply ends. */
	stream(req: StreamRequest): AsyncGenerator<StreamEvent>;
	/** Resolves if the key works; throws a ProviderError otherwise. */
	verify(key: string, signal?: AbortSignal): Promise<void>;
}

export type ErrorKind = 'auth' | 'quota' | 'model' | 'network' | 'other';

export class ProviderError extends Error {
	constructor(
		readonly kind: ErrorKind,
		message: string
	) {
		super(message);
	}
}

/** Lines of a server-sent event stream's `data:` fields, as they arrive. */
export async function* sseData(body: ReadableStream<Uint8Array>, signal: AbortSignal): AsyncGenerator<string> {
	const reader = body.getReader();
	const decoder = new TextDecoder();
	let buffer = '';
	try {
		while (!signal.aborted) {
			const { done, value } = await reader.read();
			if (done) break;
			buffer += decoder.decode(value, { stream: true });
			let nl: number;
			while ((nl = buffer.indexOf('\n')) >= 0) {
				const line = buffer.slice(0, nl).replace(/\r$/, '');
				buffer = buffer.slice(nl + 1);
				if (line.startsWith('data:')) yield line.slice(5).trimStart();
			}
		}
	} finally {
		reader.releaseLock();
	}
}
