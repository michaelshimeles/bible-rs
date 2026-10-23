// @vitest-environment happy-dom
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { ReadApp } from '../app.svelte';
import { Core } from '../core';
import { initSync } from '../wasm/bible.js';
import SourceList from './SourceList.svelte';
import { SourceListState } from './state.svelte';

let app: ReadApp;
vi.mock('../app.svelte', async (original) => ({
	...(await original<typeof import('../app.svelte')>()),
	getApp: () => app
}));

let core: Core;
let list: ReturnType<typeof mount> | undefined;

beforeAll(() => {
	initSync({ module: readFileSync(resolve('src/lib/read/wasm/bible_bg.wasm')) });
	core = new Core(async () => new Uint8Array());
});

afterEach(() => {
	if (list) unmount(list);
	list = undefined;
	document.body.innerHTML = '';
});

function reader() {
	const john = core.books.find((b) => b.name === 'John')!;
	const chapter = john.firstChapter + 2;
	app = new ReadApp({
		translations: [],
		sections: core.sections,
		translation: 'kjv',
		chapter,
		content: {
			index: chapter,
			book: john.index,
			chapter: 3,
			status: { type: 'ready' },
			bookName: john.name,
			bookTitle: john.name,
			blocks: []
		},
		label: 'John 3',
		short: 'Jn 3'
	});
	const st = new SourceListState(core.sections, john);
	list = mount(SourceList, { target: document.body, props: { st } });
	flushSync();
	return st;
}

function section(name: string) {
	const button = [...document.querySelectorAll<HTMLButtonElement>('button[aria-expanded]')].find(
		(el) => el.querySelector('.label')?.textContent === name
	);
	expect(button, `section ${name}`).toBeDefined();
	return button!;
}

function openSection(name: string) {
	section(name).click();
	flushSync();
}

function readBook(name: string) {
	app.here = { chapter: core.books.find((b) => b.name === name)!.firstChapter, verse: 1 };
	flushSync();
}

function bookNames() {
	return [...document.querySelectorAll('a.row .label')].map((el) => el.textContent);
}

describe('books by section', () => {
	it('uses plain testament headers and reveals John with its chapter grid', () => {
		const st = reader();
		expect([...document.querySelectorAll('h3')].map((el) => el.textContent)).toContain('Old Testament');
		expect([...document.querySelectorAll('h3')].map((el) => el.textContent)).toContain('New Testament');
		expect([...document.querySelectorAll('button .label')].map((el) => el.textContent)).not.toContain('New Testament');
		expect(section('Gospels and Acts').getAttribute('aria-expanded')).toBe('true');
		expect(section('Gospels and Acts').querySelector('.count')?.textContent).toBe('5');
		expect(st.autoOpened.filter(Boolean)).toHaveLength(1);
		expect(document.querySelector('a.row[aria-current] .label')?.textContent).toBe('John');
		const grid = document.querySelector('[aria-label="John chapters"]');
		expect(grid?.previousElementSibling?.querySelector('.label')?.textContent).toBe('John');
		expect(grid?.querySelector('[aria-current]')?.getAttribute('aria-label')).toBe('John 3');
	});

	it('lists every book exactly once, in canonical order, from the wasm sections', () => {
		reader();
		for (const s of core.sections) {
			if (section(s.name).getAttribute('aria-expanded') === 'false') openSection(s.name);
		}
		expect(bookNames()).toEqual(core.books.map((b) => b.name));
		expect(new Set(bookNames()).size).toBe(66);
	});

	it('closes automatic sections when reading moves and keeps manually opened sections', () => {
		reader();
		openSection('Pauline Epistles');
		readBook('Acts');
		expect(section('Gospels and Acts').getAttribute('aria-expanded')).toBe('true');
		readBook('Genesis');
		expect(section('Gospels and Acts').getAttribute('aria-expanded')).toBe('false');
		expect(section('Pentateuch').getAttribute('aria-expanded')).toBe('true');
		expect(section('Pauline Epistles').getAttribute('aria-expanded')).toBe('true');
		readBook('Romans');
		expect(section('Pentateuch').getAttribute('aria-expanded')).toBe('false');
		readBook('John');
		expect(section('Pauline Epistles').getAttribute('aria-expanded')).toBe('true');
	});

	it('closes only automatic sections off the reader and reveals the book on return', () => {
		reader();
		openSection('Pauline Epistles');
		app.page = 'notes';
		flushSync();
		expect(section('Gospels and Acts').getAttribute('aria-expanded')).toBe('false');
		expect(section('Pauline Epistles').getAttribute('aria-expanded')).toBe('true');
		app.page = 'reader';
		flushSync();
		expect(section('Gospels and Acts').getAttribute('aria-expanded')).toBe('true');
	});

	it('filters into open matching sections without changing remembered disclosures', () => {
		const st = reader();
		const remembered = [...st.sections];
		st.startFilter('  JO  ');
		flushSync();
		const matches = core.books.filter((b) => b.name.toLowerCase().includes('jo'));
		expect(bookNames()).toEqual(matches.map((b) => b.name));
		const disclosures = [...document.querySelectorAll('button[aria-expanded]')];
		expect(disclosures.map((el) => el.querySelector('.label')?.textContent)).toEqual(
			core.sections.filter((s) => s.books.some((i) => matches.some((b) => b.index === i))).map((s) => s.name)
		);
		expect(disclosures.every((el) => el.getAttribute('aria-expanded') === 'true')).toBe(true);
		expect(st.sections).toEqual(remembered);
		st.filter = 'no matching book';
		flushSync();
		expect(document.querySelector('button[aria-expanded]')).toBeNull();
		expect(document.querySelector('h3.section:not(.first)')).toBeNull();
		st.endFilter();
		flushSync();
		expect(section('Pentateuch').getAttribute('aria-expanded')).toBe('false');
		expect(section('Gospels and Acts').getAttribute('aria-expanded')).toBe('true');
	});
});
