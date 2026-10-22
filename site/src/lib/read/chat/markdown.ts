/**
 * The model's replies as a small, safe subset of Markdown: paragraphs, headings, lists and quotes,
 * with bold, italic and code inside, and Bible references as links. Nothing is rendered as HTML.
 * References are what the model writes in [[double brackets]] (as it's asked to), and any plain
 * "John 3:16" or "Psalm 23" in the text. Half-written Markdown (a reply still streaming) reads as
 * if it were closed.
 */
import { parseReference, type Book, type Reference } from '../core';

export type Inline =
	| { t: 'text'; text: string }
	| { t: 'bold'; children: Inline[] }
	| { t: 'italic'; children: Inline[] }
	| { t: 'code'; text: string }
	| { t: 'ref'; text: string; ref: Reference };

export type Block =
	| { kind: 'p'; inlines: Inline[] }
	| { kind: 'quote'; inlines: Inline[] }
	| { kind: 'h'; level: number; inlines: Inline[] }
	| { kind: 'ul'; start: number; items: Inline[][] }
	| { kind: 'ol'; start: number; items: Inline[][] };

/** Book names that are also English words: as plain text they're a reference only with a verse. */
const WORDS = new Set(['Numbers', 'Judges', 'Job', 'Mark', 'Acts', 'Ruth', 'Amos', 'Song']);
const ALIASES = ['Psalm', 'Song of Songs', 'Revelations', 'Jn', 'Mt', 'Mk', 'Lk', 'Pss'];

let cached: { books: Book[]; re: RegExp } | null = null;

/** "Book 3", "Book 3:16", "Book 3:16-18", with the book's names and abbreviations, longest first. */
function referencePattern(books: Book[]): RegExp {
	if (cached?.books === books) return cached.re;
	const names = new Set<string>(ALIASES);
	for (const b of books) {
		names.add(b.name);
		names.add(b.abbrev);
	}
	const alt = [...names]
		.sort((a, b) => b.length - a.length)
		.map((n) => n.replace(/[.*+?^${}()|[\]\\]/g, '\\$&').replace(/ /g, '\\s'))
		.join('|');
	const re = new RegExp(`(?<![\\p{L}\\d])(${alt})\\.?\\s(\\d{1,3})(?::(\\d{1,3})(?:\\s?[-–—]\\s?(\\d{1,3})(?!\\s?:))?)?(?![\\p{L}\\d:])`, 'gu');
	cached = { books, re };
	return re;
}

let whole: { books: Book[]; re: RegExp } | null = null;

/** The same, matching only a whole string (no `g`, so testing it never moves a search along). */
function wholePattern(books: Book[]): RegExp {
	if (whole?.books !== books) whole = { books, re: new RegExp(`^(?:${referencePattern(books).source})$`, 'u') };
	return whole.re;
}

/** A reference, when the text really is one (and the chapter and verse exist). */
function asReference(text: string, books: Book[]): Reference | null {
	// Only a book name and numbers: the core's reading is forgiving of typos ("link" is 2 Kings).
	if (!wholePattern(books).test(text.trim())) return null;
	const r = parseReference(text.replace(/[–—]/g, '-'));
	if (!r) return null;
	const b = books[r.book];
	if (!b || r.chapter > b.chapters) return null;
	return r;
}

/** Plain text split into text and references. */
function linkRefs(text: string, books: Book[]): Inline[] {
	if (!books.length) return text ? [{ t: 'text', text }] : [];
	const out: Inline[] = [];
	let last = 0;
	const push = (s: string) => {
		if (!s) return;
		const prev = out.at(-1);
		if (prev?.t === 'text') prev.text += s;
		else out.push({ t: 'text', text: s });
	};
	// A reference still being written ("[[Phil 4:1") reads without its brackets.
	text = text.replace(/\[\[([^\]\n]*)$/, '$1');
	// [[John 3:16]] first, then bare references in what's left.
	const bracket = /\[\[([^\]\n]{2,60})\]\]/g;
	const bare = (s: string) => {
		const re = referencePattern(books);
		re.lastIndex = 0;
		let at = 0;
		for (let m = re.exec(s); m; m = re.exec(s)) {
			const [whole, name, , verse] = m;
			if (!verse && WORDS.has(name)) continue;
			const ref = asReference(whole, books);
			if (!ref) continue;
			push(s.slice(at, m.index));
			out.push({ t: 'ref', text: whole, ref });
			at = m.index + whole.length;
		}
		push(s.slice(at));
	};
	for (let m = bracket.exec(text); m; m = bracket.exec(text)) {
		bare(text.slice(last, m.index));
		const ref = asReference(m[1], books);
		if (ref) out.push({ t: 'ref', text: m[1].trim(), ref });
		else push(m[1]);
		last = m.index + m[0].length;
	}
	bare(text.slice(last));
	return out;
}

/** Bold, italic and code spans, with references in the text between. */
export function inlines(text: string, books: Book[]): Inline[] {
	const out: Inline[] = [];
	let i = 0;
	let plain = '';
	const flush = () => {
		if (plain) out.push(...linkRefs(plain, books));
		plain = '';
	};
	while (i < text.length) {
		const c = text[i];
		if (c === '`') {
			const end = text.indexOf('`', i + 1);
			flush();
			const code = end < 0 ? text.slice(i + 1) : text.slice(i + 1, end);
			if (code) out.push({ t: 'code', text: code });
			i = end < 0 ? text.length : end + 1;
			continue;
		}
		// A link: its label (often a reference), without the address.
		if (c === '[' && text[i + 1] !== '[') {
			const m = /^\[([^\]\n]+)\]\(([^)\s]*)\)/.exec(text.slice(i));
			if (m) {
				plain += m[1];
				i += m[0].length;
				continue;
			}
		}
		const strong = text.startsWith('**', i) || text.startsWith('__', i);
		const em = !strong && (c === '*' || (c === '_' && !/[\p{L}\d]/u.test(text[i - 1] ?? '')));
		if (strong || em) {
			const marker = strong ? text.slice(i, i + 2) : c;
			const from = i + marker.length;
			let end = text.indexOf(marker, from);
			// `**` inside bold text ends it; a single `*` must close before the line does.
			if (em && end >= 0 && text[end + 1] === '*') end = -1;
			const inner = end < 0 ? text.slice(from) : text.slice(from, end);
			if (inner && !/^\s/.test(inner) && (end >= 0 || strong)) {
				flush();
				const children = inlines(inner, books);
				out.push(strong ? { t: 'bold', children } : { t: 'italic', children });
				i = end < 0 ? text.length : end + marker.length;
				continue;
			}
		}
		plain += c;
		i++;
	}
	flush();
	return out;
}

export function parse(md: string, books: Book[]): Block[] {
	const blocks: Block[] = [];
	let para: string[] = [];
	let list: { kind: 'ul' | 'ol'; start: number; items: string[] } | null = null;
	let quote: string[] = [];

	const end = () => {
		if (para.length) blocks.push({ kind: 'p', inlines: inlines(para.join(' '), books) });
		if (quote.length) blocks.push({ kind: 'quote', inlines: inlines(quote.join(' '), books) });
		if (list) {
			const items = list.items.map((t) => inlines(t, books));
			blocks.push(list.kind === 'ul' ? { kind: 'ul', start: 1, items } : { kind: 'ol', start: list.start, items });
		}
		para = [];
		quote = [];
		list = null;
	};

	for (const raw of md.replace(/\r/g, '').split('\n')) {
		const line = raw.trimEnd();
		if (!line.trim()) {
			// A blank line inside a list keeps the list going.
			if (!list) end();
			continue;
		}
		const h = /^(#{1,4})\s+(.*)$/.exec(line);
		const ul = /^\s*[-*•+]\s+(.*)$/.exec(line);
		const ol = /^\s*(\d{1,3})[.)]\s+(.*)$/.exec(line);
		const q = /^>\s?(.*)$/.exec(line);
		if (/^\s*(-{3,}|\*{3,}|_{3,})\s*$/.test(line)) {
			end();
		} else if (h) {
			end();
			blocks.push({ kind: 'h', level: h[1].length, inlines: inlines(h[2].replace(/#+$/, ''), books) });
		} else if (ul || ol) {
			const kind = ul ? 'ul' : 'ol';
			if (!list || list.kind !== kind) {
				end();
				list = { kind, start: ol ? Number(ol[1]) : 1, items: [] };
			}
			list.items.push(ul ? ul[1] : ol![2]);
		} else if (q) {
			if (!quote.length) end();
			quote.push(q[1]);
		} else if (list && /^\s{2,}/.test(raw)) {
			// A continuation line of the last item.
			list.items[list.items.length - 1] += ` ${line.trim()}`;
		} else {
			if (list || quote.length) end();
			para.push(line.trim());
		}
	}
	end();
	return blocks;
}

/** The references in a reply, in order, each once. */
export function references(blocks: Block[]): Reference[] {
	const out: Reference[] = [];
	const seen = new Set<string>();
	const walk = (xs: Inline[]) => {
		for (const x of xs) {
			if (x.t === 'ref') {
				const key = `${x.ref.chapterIndex}:${x.ref.verse ?? 0}-${x.ref.verseEnd ?? 0}`;
				if (!seen.has(key)) {
					seen.add(key);
					out.push(x.ref);
				}
			} else if (x.t === 'bold' || x.t === 'italic') walk(x.children);
		}
	};
	for (const b of blocks) {
		if (b.kind === 'ul' || b.kind === 'ol') b.items.forEach(walk);
		else walk(b.inlines);
	}
	return out;
}
