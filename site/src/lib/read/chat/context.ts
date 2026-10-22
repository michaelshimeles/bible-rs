/**
 * What a conversation is about: some verses, a chapter, a book, a note, all the notes, or
 * nothing in particular (finding verses, questions about the Bible as a whole). Each kind names
 * itself, says where it is in the reader, offers starter questions, and gives the model its text.
 */
import type { ReadApp } from '../app.svelte';
import { chapterLabel, rangeLabel, type VerseRef } from '../core';
import type { IconName } from '../ui';

export type ChatContext =
	| { kind: 'general' }
	| { kind: 'verses'; from: VerseRef; to: VerseRef }
	| { kind: 'chapter'; chapter: number }
	| { kind: 'book'; book: number }
	| { kind: 'note'; verse: VerseRef }
	| { kind: 'notes' };

/** A whole book is sent as context up to this size; beyond it, the model reads with its tools. */
const BOOK_LIMIT = 400_000;
const NOTES_LIMIT = 200;

export function contextLabel(app: ReadApp, c: ChatContext): string {
	switch (c.kind) {
		case 'general':
			return 'The whole Bible';
		case 'verses':
			return rangeLabel(c.from, c.to);
		case 'chapter':
			return chapterLabel(c.chapter);
		case 'book':
			return app.core?.books[c.book]?.name ?? 'Book';
		case 'note':
			return `Note on ${app.label(c.verse)}`;
		case 'notes':
			return 'My notes';
	}
}

/** A word on what the context is, under its label. */
export function contextKind(c: ChatContext): string {
	switch (c.kind) {
		case 'general':
			return 'Find verses, ask anything';
		case 'verses':
			return c.from.verse === c.to.verse ? 'Verse' : 'Passage';
		case 'chapter':
			return 'Chapter';
		case 'book':
			return 'Book';
		case 'note':
			return 'Note';
		case 'notes':
			return 'All notes';
	}
}

export function contextIcon(c: ChatContext): IconName {
	switch (c.kind) {
		case 'general':
			return 'sparkle';
		case 'verses':
			return 'quote';
		case 'chapter':
			return 'doc';
		case 'book':
			return 'book';
		case 'note':
		case 'notes':
			return 'note';
	}
}

/** Where the context is in the reader, if it's somewhere. */
export function contextPlace(app: ReadApp, c: ChatContext): { chapter: number; verse?: number } | null {
	switch (c.kind) {
		case 'verses':
			return { chapter: app.chapterIndex(c.from), verse: c.from.verse };
		case 'note':
			return { chapter: app.chapterIndex(c.verse), verse: c.verse.verse };
		case 'chapter':
			return { chapter: c.chapter };
		case 'book': {
			const b = app.core?.books[c.book];
			return b ? { chapter: b.firstChapter } : null;
		}
		default:
			return null;
	}
}

export const sameContext = (a: ChatContext, b: ChatContext) => JSON.stringify(a) === JSON.stringify(b);

/** Starter questions for an empty conversation. */
export function suggestions(c: ChatContext): string[] {
	switch (c.kind) {
		case 'general':
			return [
				'Find me a verse to help me stay motivated',
				'Verses for when I feel anxious',
				'What does the Bible say about forgiveness?',
				'Help me through a hard time'
			];
		case 'verses':
			return [
				'What does this mean?',
				'What was happening when this was written?',
				'How can I apply this today?',
				'Where else does the Bible say this?'
			];
		case 'chapter':
			return [
				'Summarize this chapter',
				'What are the key themes?',
				'Who are the people in this chapter?',
				'What can I take from this today?'
			];
		case 'book':
			return [
				'Give me an overview of this book',
				'Who wrote it, and when?',
				'What are its main themes?',
				'Which chapters should I read first?'
			];
		case 'note':
			return ['Help me go deeper on this', 'Is my reading of this verse right?', 'Find related verses', 'Turn this into a prayer'];
		case 'notes':
			return [
				'What themes run through my notes?',
				'Summarize what I have been studying',
				'Suggest what to read next',
				'Find verses that connect my notes'
			];
	}
}

const verseText = (app: ReadApp, id: string, v: VerseRef) =>
	app.core?.verse(id, v)?.runs.map((r) => r.text).join('').trim() ?? '';

/** A chapter's verses, one per line with their numbers. */
function chapterText(app: ReadApp, id: string, index: number): string {
	const verses = app.verses(id, index) ?? [];
	return verses
		.map((v) => {
			const n = v.numberEnd > v.number ? `${v.number}-${v.numberEnd}` : `${v.number}`;
			return `${n} ${v.runs.map((r) => r.text).join('').trim()}`;
		})
		.join('\n');
}

/**
 * The context as text for the model, in translation `id` (loaded first). Verses come with their
 * chapter around them; a book whole, when it isn't too long; notes with the verses they're on.
 */
export async function contextText(app: ReadApp, c: ChatContext, id: string): Promise<string> {
	await app.ensure(id);
	const abbrev = app.info(id)?.abbrev ?? id.toUpperCase();
	switch (c.kind) {
		case 'general':
			return '';
		case 'verses': {
			const index = app.chapterIndex(c.from);
			const picked: string[] = [];
			for (let n = c.from.verse; n <= c.to.verse; n++) {
				const t = verseText(app, id, { ...c.from, verse: n });
				if (t) picked.push(`${n} ${t}`);
			}
			return [
				`The reader selected ${rangeLabel(c.from, c.to)} (${abbrev}) and is asking about it:`,
				picked.join('\n'),
				'',
				`The whole chapter, ${chapterLabel(index)}, for context:`,
				chapterText(app, id, index)
			].join('\n');
		}
		case 'chapter':
			return [`The reader is asking about ${chapterLabel(c.chapter)} (${abbrev}):`, chapterText(app, id, c.chapter)].join('\n');
		case 'book': {
			const b = app.core?.books[c.book];
			if (!b) return '';
			const parts: string[] = [];
			let size = 0;
			for (let k = 0; k < b.chapters; k++) {
				const text = `## ${b.name} ${k + 1}\n${chapterText(app, id, b.firstChapter + k)}`;
				size += text.length;
				if (size > BOOK_LIMIT) {
					parts.push(`(The rest of ${b.name} is left out for length; read it with the read_passage tool.)`);
					break;
				}
				parts.push(text);
			}
			return [`The reader is asking about the book of ${b.name} (${b.chapters} chapters, ${abbrev}). Its text:`, ...parts].join(
				'\n\n'
			);
		}
		case 'note': {
			const body = app.noteFor(c.verse)?.body ?? '';
			return [
				`The reader wrote a note on ${app.label(c.verse)} (${abbrev}): "${verseText(app, id, c.verse)}"`,
				'Their note (Markdown):',
				body || '(empty)'
			].join('\n');
		}
		case 'notes': {
			const notes = app.studyItems('notes').slice(0, NOTES_LIMIT);
			if (!notes.length) return 'The reader has not written any notes yet.';
			return [
				`The reader's study notes (${notes.length}), each on a verse (${abbrev}):`,
				...notes.map((n) => `### ${n.reference}: "${n.text.trim()}"\n${n.body.trim()}`)
			].join('\n\n');
		}
	}
}
