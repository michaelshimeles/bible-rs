import type { StudyKind } from '../app.svelte';
import type { BibleSection, Book, HighlightColor } from '../core';
import type { BookGrouping } from '../settings';

export const STUDY_KINDS: StudyKind[] = ['bookmarks', 'highlights', 'notes'];

/** The disclosures the books sit under: the canon's sections, or the testaments as runs of books
 *  like them (sidebar.rs `BookGrouping::groups`). */
export function bookGroups(grouping: BookGrouping, sections: BibleSection[]): BibleSection[] {
	if (grouping === 'section') return sections;
	const testament = (newTestament: boolean) => ({
		name: newTestament ? 'New Testament' : 'Old Testament',
		books: sections.filter((s) => s.newTestament === newTestament).flatMap((s) => s.books),
		newTestament
	});
	return [testament(false), testament(true)];
}

/** What the source list remembers while search takes its place (sidebar.rs `Sidebar`). */
export class SourceListState {
	/** How the books are grouped, the groups in canon order, and which of them are open. */
	grouping = $state<BookGrouping>('testament');
	groups = $state.raw<BibleSection[]>([]);
	open = $state<boolean[]>([]);
	/** Groups opened to reveal the book being read, which close again when it leaves them. */
	autoOpened: boolean[] = [];
	/** The book whose chapters show. */
	expanded = $state<number | null>(null);
	/** The book last revealed (its group opened and it scrolled into view). */
	revealed: number | null = null;
	/** Study collections listing their recent items in place, and those showing all of them. */
	studyOpen = $state([false, false, false]);
	studyAll = $state([false, false, false]);
	highlightFilter = $state<HighlightColor | null>(null);
	/** Type-to-filter over the books. */
	filtering = $state(false);
	filter = $state('');
	/** Bumped to focus the filter field. */
	filterFocus = $state(0);
	readonly #sections: BibleSection[];

	/** Opens on the book being read, as SourceList reveals it, so the page arrives with its chapters
	 *  listed (and linked) rather than opening them once it has loaded. */
	constructor(sections: BibleSection[], grouping: BookGrouping, reading?: Book) {
		this.#sections = sections;
		this.regroup(grouping);
		if (!reading) return;
		const g = this.groups.findIndex((g) => g.books.includes(reading.index));
		if (g >= 0) {
			this.open[g] = true;
			this.autoOpened[g] = true;
		}
		this.expanded = reading.index;
	}

	/** Groups the books afresh, all closed, for SourceList to reveal the book being read again. */
	regroup(grouping: BookGrouping) {
		this.grouping = grouping;
		this.groups = bookGroups(grouping, this.#sections);
		this.open = this.groups.map(() => false);
		this.autoOpened = this.groups.map(() => false);
		this.revealed = null;
	}

	toggleStudy(kind: StudyKind) {
		const i = STUDY_KINDS.indexOf(kind);
		this.studyOpen[i] = !this.studyOpen[i];
	}

	startFilter(text: string) {
		this.filtering = true;
		this.filter = text;
		this.filterFocus++;
	}

	endFilter() {
		this.filtering = false;
		this.filter = '';
	}
}
