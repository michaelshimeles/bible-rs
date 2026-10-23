import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { Action, ReadApp } from './app.svelte';
import { handleKey } from './keyboard';

/** The focused element; `closest` knows the classes of the book picker's panel when `inPicker`. */
function element(inPicker: boolean) {
	const classes = inPicker ? ['.popover', '.book-picker'] : [];
	return { tagName: 'BUTTON', isContentEditable: false, closest: (s: string) => (classes.includes(s) ? {} : null) };
}

let focused: ReturnType<typeof element> | null = null;
let dispatched: Action['type'][] = [];

const app = {
	onboarding: false,
	settings: { shortcuts: {} },
	palette: null,
	settingsTab: null,
	shortcuts: false,
	share: null,
	importSheet: null,
	selectedRefs: () => [],
	dispatch: (a: Action) => dispatched.push(a.type)
} as unknown as ReadApp;

/** Presses the key at `code` (typing `key`) and returns what the reader did with it. */
function press(code: string, key: string, held: Partial<KeyboardEvent> = {}): string {
	dispatched = [];
	const e = {
		code,
		key,
		metaKey: false,
		ctrlKey: false,
		altKey: false,
		shiftKey: false,
		isComposing: false,
		defaultPrevented: false,
		preventDefault: () => {},
		...held
	} as unknown as KeyboardEvent;
	return handleKey(app, e) ? dispatched.join() : 'ignored';
}

const KEYS: [string, string, string][] = [
	['KeyH', 'h', 'prevChapter'],
	['KeyL', 'l', 'nextChapter'],
	['BracketLeft', '[', 'prevChapter'],
	['BracketRight', ']', 'nextChapter']
];

beforeEach(() => {
	focused = null;
	vi.stubGlobal('document', {
		get activeElement() {
			return focused;
		}
	});
});

describe('H, L, [ and ]', () => {
	it('turn chapters like ← and →', () => {
		for (const [code, key, turn] of KEYS) expect(press(code, key), key).toBe(turn);
	});

	it('only bare: not with Shift, Alt, Control or Command held', () => {
		for (const [code, key] of KEYS) {
			for (const held of ['shiftKey', 'altKey', 'ctrlKey', 'metaKey']) {
				expect(press(code, key, { [held]: true }), `${held} ${key}`).not.toMatch(/Chapter/);
			}
		}
	});

	it('wait while the book picker has the keyboard, as ← and → do', () => {
		focused = element(true);
		for (const [code, key] of KEYS) expect(press(code, key), key).toBe('ignored');
		focused = element(false);
		expect(press('KeyL', 'l')).toBe('nextChapter');
	});
});
