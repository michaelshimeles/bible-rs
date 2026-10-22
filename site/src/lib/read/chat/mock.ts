/**
 * Development only (demo.ts): a stand-in for the provider that streams written replies, with a
 * tool call where a real model would make one, so the chat can be shown and screenshotted
 * without an API key. Never part of the reader's build for visitors.
 */
import type { Provider, StreamEvent, StreamRequest } from './provider';

interface Scripted {
	match: RegExp;
	tool?: { name: string; args: Record<string, string> };
	reply: string;
}

const SCRIPT: Scripted[] = [
	{
		match: /motivat/i,
		tool: { name: 'search_bible', args: { query: 'renew strength' } },
		reply: `Here are a few passages to keep you going when your motivation runs low:

- **[[Isaiah 40:31]]** — Strength is promised to those who *wait upon the LORD*: not a burst of willpower, but renewed endurance, like an eagle riding the wind.
- **[[Philippians 4:13]]** — Paul wrote this from prison. The strength he means is Christ's, given for whatever the day holds.
- **[[Galatians 6:9]]** — Keep doing good even when you can't see results yet: "in due season we shall reap, if we faint not."
- **[[Joshua 1:9]]** — Courage that rests on God's presence: "the LORD thy God is with thee whithersoever thou goest."

Pick one and read it each morning this week. Would you like verses for something specific, like work, health or a goal you're chasing?`
	},
	{
		match: /anxious|anxiety|worry/i,
		tool: { name: 'search_bible', args: { query: 'careful nothing' } },
		reply: `Anxiety is something Scripture speaks to gently and often:

- **[[Philippians 4:6-7]]** — Bring everything to God in prayer, *with thanksgiving*, and his peace will guard your heart and mind.
- **[[1 Peter 5:7]]** — "Casting all your care upon him; for he careth for you."
- **[[Matthew 6:34]]** — Jesus invites you to live one day at a time.
- **[[Psalm 94:19]]** — Even "in the multitude of my thoughts within me," God's comforts delight the soul.

You're not alone in this. If the worry feels overwhelming, it's also good to talk to someone you trust.`
	},
	{
		match: /mean/i,
		reply: `**[[John 3:16]]** is often called the gospel in a sentence. Jesus says it to Nicodemus, a Pharisee who came to him by night ([[John 3:1-2]]).

- **"God so loved the world"** — *so* means "in this way": the love is shown by what God does. And *the world* is everyone, not one nation.
- **"he gave his only begotten Son"** — the gift points to the cross. Just before, Jesus compares himself to the serpent Moses lifted up in the wilderness ([[John 3:14-15]], [[Numbers 21:8-9]]).
- **"whosoever believeth"** — the offer is open to anyone. To believe here is to trust, not only to agree.
- **"should not perish, but have everlasting life"** — in John, eternal life is a new kind of life that begins now ([[John 17:3]]).

The next verse makes the purpose plain: God sent his Son "not to condemn the world; but that the world through him might be saved" ([[John 3:17]]).`
	},
	{
		match: /apply|today/i,
		reply: `A few ways to live **[[John 3:16]]** this week:

1. **Receive it personally.** Read it with your own name in place of "the world". The love is meant for you.
2. **Trust, don't just agree.** Belief in John is leaning your weight on Jesus. Where are you carrying something alone?
3. **Pass it on.** God's love moved toward people. Who could you show it to in a practical way?

[[1 John 4:9-11]] draws the same line: "if God so loved us, we ought also to love one another."`
	},
	{
		match: /summar/i,
		reply: `**[[Psalm 23]]** is David's song of trust in God as his shepherd.

- **Provision (vv. 1–3):** green pastures, still waters, a restored soul. "I shall not want" means *I lack nothing I need*.
- **Presence (v. 4):** in "the valley of the shadow of death" the psalm turns from *he* to *you*: "thou art with me."
- **Honour (vv. 5–6):** the shepherd becomes a host who sets a table even before enemies, and goodness and mercy *follow* — the Hebrew word means pursue.

Jesus takes up the image when he calls himself the good shepherd who lays down his life for the sheep ([[John 10:11]]).`
	},
	{
		match: /theme|notes/i,
		reply: `Two threads run through your notes:

1. **God coming near.** Your note on [[John 1:14]] links the Word becoming flesh with [[Philippians 2:6-8]] and [[Hebrews 1:3]] — God doesn't stay distant; he takes on our life.
2. **God at work in hard things.** On [[Romans 8:28]] you wrote that God works *all things together* for good, not that all things are good, and tied it to Joseph in [[Genesis 50:20]].

Together they say: God is *with* you and *for* you. A good next read would be [[Romans 8:31-39]], which brings both together.`
	}
];

const FALLBACK = `That's a good question. Here's what Scripture says about it, briefly:

- **[[Proverbs 3:5-6]]** — Trust the LORD with all your heart, and he will direct your paths.
- **[[James 1:5]]** — If you lack wisdom, ask God, "that giveth to all men liberally."

Tell me a little more and I can point you to passages that fit your situation.`;

const sleep = (ms: number, signal: AbortSignal) =>
	new Promise<void>((resolve, reject) => {
		const t = setTimeout(resolve, ms);
		signal.addEventListener('abort', () => (clearTimeout(t), reject(new DOMException('Aborted', 'AbortError'))), { once: true });
	});

export interface MockOptions {
	/** Milliseconds between chunks. */
	delay: number;
	/** Stops this many characters in and waits (a screenshot of a reply arriving). */
	freezeAt: number | null;
	/** Fails with this kind of error. */
	fail: 'auth' | null;
}

export function mockProvider(base: Provider, options: MockOptions): Provider {
	return {
		...base,
		async verify(key) {
			if (!key.startsWith('sk-')) throw new (await import('./provider')).ProviderError('auth', "OpenAI didn't accept this API key. Check it, or make a new one.");
		},
		async *stream({ messages, signal }: StreamRequest): AsyncGenerator<StreamEvent> {
			if (options.fail) {
				await sleep(300, signal);
				const { ProviderError } = await import('./provider');
				throw new ProviderError('auth', "OpenAI didn't accept this API key. Check it, or make a new one.");
			}
			const users = messages.filter((m) => m.role === 'user');
			const question = users.at(-1)?.content ?? '';
			const s = SCRIPT.find((x) => x.match.test(String(question)));
			const last = messages.at(-1);
			await sleep(Math.max(150, options.delay * 10), signal);
			if (s?.tool && last?.role === 'user') {
				yield { type: 'toolCalls', calls: [{ id: 'call_1', name: s.tool.name, args: JSON.stringify(s.tool.args) }] };
				return;
			}
			const reply = s?.reply ?? FALLBACK;
			let at = 0;
			while (at < reply.length) {
				if (options.freezeAt !== null && at >= options.freezeAt) await sleep(1e9, signal);
				const n = 3 + Math.floor(Math.random() * 6);
				yield { type: 'text', text: reply.slice(at, at + n) };
				at += n;
				if (options.delay) await sleep(options.delay, signal);
			}
		}
	};
}
