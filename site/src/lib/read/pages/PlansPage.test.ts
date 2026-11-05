// @vitest-environment happy-dom
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { ReadApp } from '../app.svelte';
import { Core } from '../core';
import { wasmBytes } from '../node';
import { initSync } from '../wasm/bible.js';
import PlansPage from './PlansPage.svelte';
import { plansPage } from './plans.svelte';

let app: ReadApp;
vi.mock('../app.svelte', async (original) => ({
	...(await original<typeof import('../app.svelte')>()),
	getApp: () => app
}));

beforeAll(() => {
	initSync({ module: wasmBytes() });
});

let page: ReturnType<typeof mount> | undefined;

afterEach(() => {
	if (page) unmount(page);
	page = undefined;
	document.body.innerHTML = '';
	plansPage.tab = 'active';
	plansPage.open = null;
	plansPage.editor = null;
	plansPage.confirm = null;
	plansPage.leaving = null;
});

function reader() {
	const core = new Core(async () => new Uint8Array());
	app = new ReadApp({
		translations: [],
		sections: core.sections,
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
	app.core = core;
	return app;
}

function startGospel() {
	reader();
	app.dispatch({ type: 'startPlan', id: 'john-21' });
	app.dispatch({ type: 'setPlanRead', plan: 'john-21', day: 1, passage: 0, read: true });
	expect(app.plans().some((p) => p.id === 'john-21' && p.finished === null)).toBe(true);
	expect([...app.planReads('john-21')]).toContain('1:0');
	plansPage.tab = 'active';
	plansPage.open = null;
	plansPage.leaving = null;
}

function showList() {
	page = mount(PlansPage, { target: document.body });
	flushSync();
}

function showDay() {
	plansPage.open = { id: 'john-21', day: 1 };
	page = mount(PlansPage, { target: document.body });
	flushSync();
}

function button(label: string) {
	const found = [...document.querySelectorAll('button')].filter((el) => el.textContent?.trim() === label);
	expect(found, `button ${label}`).toHaveLength(1);
	return found[0];
}

function shown(text: string) {
	return (document.body.textContent ?? '').includes(text);
}

function left() {
	expect(app.plans().some((p) => p.id === 'john-21')).toBe(false);
	expect([...app.planReads('john-21')]).toHaveLength(0);
}

describe('leaving a reading plan', () => {
	it('asks on the Active card, then clears the plan and its ticks', () => {
		startGospel();
		showList();
		expect(shown('Gospel of John')).toBe(true);
		button('Leave plan').click();
		flushSync();
		expect(shown('This clears your reading ticks.')).toBe(true);
		expect(app.plans().some((p) => p.id === 'john-21')).toBe(true);
		expect([...app.planReads('john-21')]).toContain('1:0');
		button('Leave plan').click();
		flushSync();
		left();
		expect(shown('This clears your reading ticks.')).toBe(false);
	});

	it('keeps the plan when the confirm is cancelled', () => {
		startGospel();
		showList();
		button('Leave plan').click();
		flushSync();
		button('Cancel').click();
		flushSync();
		expect(shown('This clears your reading ticks.')).toBe(false);
		expect(app.plans().some((p) => p.id === 'john-21')).toBe(true);
		expect([...app.planReads('john-21')]).toContain('1:0');
	});

	it('asks on the plan page, then returns to the list', () => {
		startGospel();
		showDay();
		button('Leave…').click();
		flushSync();
		expect(shown('Leave this plan?')).toBe(true);
		expect(app.plans().some((p) => p.id === 'john-21')).toBe(true);
		button('Leave plan').click();
		flushSync();
		left();
		expect(plansPage.open).toBeNull();
		expect(plansPage.leaving).toBeNull();
	});

	it('leaves a custom plan without deleting the definition', () => {
		reader();
		app.dispatch({
			type: 'saveCustomPlan',
			id: '',
			title: 'Wisdom',
			description: 'A chapter a day',
			days: ['John 1']
		});
		const mine = app.customPlans();
		expect(mine).toHaveLength(1);
		const id = mine[0].id;
		app.dispatch({ type: 'startPlan', id });
		app.dispatch({ type: 'setPlanRead', plan: id, day: 1, passage: 0, read: true });
		expect(app.plans().some((p) => p.id === id && p.finished === null)).toBe(true);
		plansPage.tab = 'active';
		plansPage.open = null;
		plansPage.leaving = null;
		showList();
		expect(shown('Wisdom')).toBe(true);
		button('Leave plan').click();
		flushSync();
		expect(shown('This clears your reading ticks.')).toBe(true);
		expect(app.customPlans().some((p) => p.id === id)).toBe(true);
		button('Leave plan').click();
		flushSync();
		expect(app.plans().some((p) => p.id === id)).toBe(false);
		expect([...app.planReads(id)]).toHaveLength(0);
		expect(app.customPlans().some((p) => p.id === id && p.title === 'Wisdom')).toBe(true);
	});
});
