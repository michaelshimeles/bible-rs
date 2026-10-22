// See https://svelte.dev/docs/kit/types#app.d.ts
// for information about these interfaces
declare global {
	namespace App {
		// interface Error {}
		// interface Locals {}
		// interface PageData {}
		interface PageState {
			/** The reader's place in its own history (Shell.svelte): entries count up from 0. */
			nav?: number;
		}
		// interface Platform {}
	}
}

export {};
