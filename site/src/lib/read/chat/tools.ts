/**
 * Tools the model can call, run here against the reader's own text: a word search across the
 * translation (the reader's search, so it finds what's really in the text) and reading a passage.
 */
import type { ReadApp } from '../app.svelte';
import { chapterLabel, parseReference, type SearchScope } from '../core';
import type { ToolSpec } from './provider';

export const TOOLS: ToolSpec[] = [
	{
		name: 'search_bible',
		description:
			"Searches the reader's Bible translation for verses containing every given word (word prefixes match, so 'strength' also finds 'strengthen'). Use one to three distinctive words, not a sentence. Returns up to 15 verses with their references.",
		parameters: {
			type: 'object',
			properties: {
				query: { type: 'string', description: 'One to three words, e.g. "strength weary" or "fear not". "Quoted phrases" match in order.' },
				scope: {
					type: 'string',
					description: 'Optional: "ot", "nt", or a book name such as "Psalms". Leave out for the whole Bible.'
				}
			},
			required: ['query'],
			additionalProperties: false
		}
	},
	{
		name: 'read_passage',
		description:
			"Reads a passage from the reader's translation: a verse (\"Isaiah 40:31\"), a range (\"Romans 8:28-39\") or a whole chapter (\"Psalm 23\").",
		parameters: {
			type: 'object',
			properties: { reference: { type: 'string', description: 'A Bible reference.' } },
			required: ['reference'],
			additionalProperties: false
		}
	}
];

export interface ToolResult {
	/** What the model reads. */
	content: string;
	/** What the reader sees in the conversation ("Searched for “strength” · 12 verses"). */
	label: string;
}

const MAX_HITS = 15;
const MAX_VERSES = 60;

function args(json: string): Record<string, unknown> {
	try {
		const v = JSON.parse(json || '{}');
		return typeof v === 'object' && v ? v : {};
	} catch {
		return {};
	}
}

/** "Searching for “strength”…", while the tool runs. */
export function toolPending(name: string, json: string): string {
	const a = args(json);
	if (name === 'search_bible') return `Searching for “${String(a.query ?? '')}”…`;
	if (name === 'read_passage') return `Reading ${String(a.reference ?? '')}…`;
	return 'Working…';
}

export async function runTool(app: ReadApp, translation: string, name: string, json: string): Promise<ToolResult> {
	const a = args(json);
	const core = app.core;
	if (!core || !(await app.ensure(translation))) return { content: 'The Bible text is not available right now.', label: 'Bible text unavailable' };
	const abbrev = app.info(translation)?.abbrev ?? translation.toUpperCase();

	if (name === 'search_bible') {
		const query = String(a.query ?? '').trim();
		if (!query) return { content: 'Give a query.', label: 'Search' };
		let scope: SearchScope = 'all';
		const s = String(a.scope ?? '').trim().toLowerCase();
		if (s === 'ot' || s === 'nt') scope = s;
		else if (s) {
			const r = parseReference(`${s} 1`);
			if (r) scope = r.book;
		}
		const page = core.search(translation, query, scope, false, null, 0, MAX_HITS);
		const lines = page.hits.map((h) => `${h.reference}: ${h.text}`);
		const more = page.total > page.hits.length ? `\n(${page.total - page.hits.length} more matches not shown; narrow the search to see others.)` : '';
		const found = page.total === 1 ? '1 verse' : `${page.total.toLocaleString('en-US')} verses`;
		return {
			content: page.total
				? `${found} in the ${abbrev} contain “${query}”:\n${lines.join('\n')}${more}`
				: `No verses in the ${abbrev} contain “${query}”${page.suggestion ? `; “${page.suggestion.query}” finds ${page.suggestion.total}` : ''}. Try other words.`,
			label: `Searched for “${query}” · ${page.total ? found : 'nothing found'}`
		};
	}

	if (name === 'read_passage') {
		const text = String(a.reference ?? '');
		const r = parseReference(text);
		if (!r) return { content: `“${text}” isn't a reference I can read.`, label: `Couldn't read “${text}”` };
		const verses = app.verses(translation, r.chapterIndex) ?? [];
		const from = r.verse ?? 1;
		const to = r.verse ? (r.verseEnd ?? r.verse) : Infinity;
		const picked = verses.filter((v) => v.numberEnd >= from && v.number <= to).slice(0, MAX_VERSES);
		const body = picked.map((v) => `${v.number} ${v.runs.map((x) => x.text).join('').trim()}`).join('\n');
		const label = r.verse ? r.label : chapterLabel(r.chapterIndex);
		return {
			content: body ? `${label} (${abbrev}):\n${body}` : `${label} isn't in the ${abbrev}.`,
			label: `Read ${label}`
		};
	}

	return { content: `There's no tool called ${name}.`, label: name };
}
