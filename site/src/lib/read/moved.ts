/**
 * What the reader says after static/move/move.js brought a library over from the old address:
 * once, the first time it opens afterwards.
 */
export const NOTICE_KEY = 'bible.read.movedNotice';

interface Notice {
	from: string;
	highlights: number;
	notes: number;
	bookmarks: number;
	translations: string[];
}

const count = (n: number, one: string) => `${n} ${one}${n === 1 ? '' : 's'}`;

/** The toasts for a notice: what came over, then the translations that didn't. */
export function movedText(n: Notice): string[] {
	const parts = [count(n.highlights, 'highlight'), count(n.notes, 'note'), count(n.bookmarks, 'bookmark')];
	const out = [`Brought ${parts.join(', ')} over from ${n.from}`];
	if (n.translations.length) out.push(`Import ${n.translations.join(', ')} again: imported translations stay on ${n.from}`);
	return out;
}

/** Takes the notice, if there is one, so it shows once. */
export function takeMovedNotice(storage: Storage): string[] {
	try {
		const raw = storage.getItem(NOTICE_KEY);
		if (raw === null) return [];
		storage.removeItem(NOTICE_KEY);
		const n = JSON.parse(raw);
		if (!n || typeof n.from !== 'string') return [];
		const num = (v: unknown) => (typeof v === 'number' && v >= 0 ? Math.floor(v) : 0);
		return movedText({
			from: n.from,
			highlights: num(n.highlights),
			notes: num(n.notes),
			bookmarks: num(n.bookmarks),
			translations: Array.isArray(n.translations) ? n.translations.filter((t: unknown) => typeof t === 'string') : []
		});
	} catch {
		return [];
	}
}
