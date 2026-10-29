// @vitest-environment happy-dom
import { flushSync, mount, tick, unmount } from 'svelte';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { ReadApp } from '../app.svelte';
import type { Core } from '../core';
import { wasmBytes } from '../node';
import * as wasm from '../wasm/bible.js';
import BookPicker from './BookPicker.svelte';

/** The app the picker reads (getApp's context comes from Shell.svelte in the reader). */
let app: ReadApp;
vi.mock('../app.svelte', async (original) => ({
	...(await original<typeof import('../app.svelte')>()),
	getApp: () => app
}));

beforeAll(() => {
	wasm.initSync({ module: wasmBytes() });
});

/** Reading Genesis 1, with the core's books. */
function reader(): ReadApp {
	const app = new ReadApp({
		sections: [],
		translations: [],
		translation: 'kjv',
		chapter: 0,
		content: {
			index: 0,
			book: 1,
			chapter: 1,
			status: { type: 'ready' },
			bookName: 'Genesis',
			bookTitle: 'Genesis',
			blocks: []
		},
		label: 'Genesis 1',
		short: 'Gen 1'
	});
	app.core = { books: wasm.books(), sections: wasm.bibleSections() } as unknown as Core;
	return app;
}

let picker: ReturnType<typeof mount> | undefined;

afterEach(() => {
	if (picker) unmount(picker);
	picker = undefined;
	document.body.innerHTML = '';
});

/** Opens the picker and returns where its keys go and the jumps it asks for. */
async function open() {
	app = reader();
	const jumps: unknown[] = [];
	vi.spyOn(app, 'dispatch').mockImplementation((a) => void jumps.push(a));
	const anchor = document.createElement('button');
	document.body.append(anchor);
	picker = mount(BookPicker, { target: document.body, props: { anchor, onclose: () => {} } });
	flushSync();
	await tick();
	const panel = document.querySelector<HTMLElement>('[role="menu"]')!;
	expect(panel, 'the picker is open').not.toBeNull();
	const press = async (...keys: string[]) => {
		for (const key of keys) {
			const target = (document.activeElement as HTMLElement | null) ?? panel;
			target.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true }));
			flushSync();
			await tick();
			flushSync();
			const up = (document.activeElement as HTMLElement | null) ?? panel;
			up.dispatchEvent(new KeyboardEvent('keyup', { key, bubbles: true, cancelable: true }));
			flushSync();
			await tick();
			flushSync();
		}
	};
	const lit = () => (document.activeElement as HTMLElement | null)?.textContent?.trim();
	return { press, lit, jumps };
}

const chapter = (name: string, n: number) => {
	const b = wasm.books().find((b: { name: string }) => b.name === name)!;
	return b.firstChapter + n - 1;
};

describe('BookPicker typing', () => {
	it('opens a typed chapter in the lit book, not the one first matched', async () => {
		const { press, lit, jumps } = await open();
		await press('j', 'o', ' ', '3');
		expect(lit()).toBe('John');
		await press('ArrowDown');
		expect(lit()).toBe('Job');
		await press('Enter');
		expect(jumps).toEqual([{ type: 'jump', chapter: chapter('Job', 3), verse: undefined, flash: false }]);
	});

	it('starts a chapter number afresh after an arrow moves the highlight', async () => {
		const { press, lit, jumps } = await open();
		// Genesis' chapters: 1 lights chapter 1, right moves to 2, then 2 lights chapter 2, not 12.
		await press('g', 'e', 'n', 'Enter', '1');
		expect(lit()).toBe('1');
		await press('ArrowRight');
		expect(lit()).toBe('2');
		await press('2');
		expect(lit()).toBe('2');
		expect(jumps).toEqual([]);
	});
});
