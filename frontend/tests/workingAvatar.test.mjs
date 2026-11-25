// The bot at work (autonomy/working-avatar/design.md §10): the scene that stands in the live bubble before the first words.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { loadBundled } from './bundle.mjs';

const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { WorkingAvatar } from './src/components/WorkingAvatar';
  export const scene = (name, props = {}) => renderToStaticMarkup(React.createElement(WorkingAvatar, { name, ...props }));
`);
const scene = ui.scene;
const read = (p) => readFileSync(new URL(`../${p}`, import.meta.url), 'utf8');
const css = read('src/styles/stream.css');
const tsx = read('src/components/WorkingAvatar.tsx');
const chat = read('src/components/ChatView.tsx');

const body = (html) => html.match(/<rect x="7" y="9" width="12" height="7" fill="([^"]+)"/)?.[1];
const paw = (html) => html.match(/<rect x="13" y="14" width="3" height="1" fill="([^"]+)" data-paw=""/)?.[1];

test('36d: one polite region says line 1 as a sentence; the scene, line 1 and the clock are hidden from screen readers', () => {
  const html = scene('Builder');
  assert.match(html, /^<div class="kw-wa-card" data-working-avatar="">/);
  assert.doesNotMatch(html, /aria-label="Builder is working on the answer"/, 'no fixed label any more');
  assert.equal((html.match(/role="status"/g) ?? []).length, 1, 'one region per turn');
  assert.match(html, /<p class="sr-only" role="status" aria-live="polite" data-scene-said="">Builder is getting started\.<\/p>/);
  assert.match(html, /<svg class="kw-wa-scene"[^>]*aria-hidden="true"[^>]*focusable="false"/);
  assert.doesNotMatch(html, /role="img"/);
  assert.match(html, /<p class="kw-wa-now[^"]*" aria-hidden="true" data-scene-line=""><span class="font-medium text-zinc-50">Builder<\/span> is getting started<\/p>/);
  assert.match(html, /<span class="tabular-nums" aria-hidden="true" data-working-elapsed="">Working · 0:0\d<\/span>/);
  assert.doesNotMatch(html, /kw-mood-dot|kw-wa-dots/, 'the three dots go: line 1 ends in a real object');
  assert.doesNotMatch(html, /<button/, 'no steps yet: nothing to open');
});

test('36d: a shell step names the host in bright type, draws the command glyph, and the Steps toggle controls its list', () => {
  const live = { turnId: 't1', thinking: {}, steps: [
    { id: 's1', kind: 'tool', status: 'failed', title: 'Running `ssh root@203.0.113.66 uptime`', started_at: '2026-11-25T00:00:00Z' },
    { id: 's2', kind: 'tool', status: 'running', title: 'Running `ssh root@203.0.113.66 ls state/`', started_at: '2026-11-25T00:00:03Z' },
  ] };
  const html = scene('Quant', { live, idBase: 'live-t1', footer: 'Runs on Claude Code CLI · this machine' });
  assert.match(html, /data-scene-line=""><span class="font-medium text-zinc-50">Quant<\/span> is running a command on <span class="font-medium text-zinc-50">203\.0\.113\.66<\/span><\/p>/);
  assert.match(html, /data-scene-said="">Quant is running a command on 203\.0\.113\.66\.<\/p>/);
  assert.match(html, /data-glyph="command"/);
  for (const [x, y] of [[19, 8], [20, 9], [19, 10]]) assert.match(html, new RegExp(`<rect x="${x}" y="${y}" width="1" height="1" fill="var\\(--kw-wa-type\\)"`));
  assert.match(html, /class="kw-wa-l kw-wa-l1" x="22" y="9" width="9"/);
  assert.match(html, /class="kw-wa-l kw-wa-l3" x="19" y="12" width="7"/);
  assert.match(html, /<button type="button" aria-expanded="false" aria-controls="live-t1-steps" aria-label="Steps, 2, 1 retried" data-scene-steps="" class="min-h-7 /);
  assert.match(html, /<div id="live-t1-steps" hidden="" class="border-t border-hairline" data-scene-steps-list="">/);
  assert.match(html, /data-scene-footer="">Runs on Claude Code CLI · this machine<\/p>/, 'the engine note is the list footer');
  const phone = scene('Quant', { live, phone: true });
  assert.match(phone, /width="72" height="36"/);
  assert.match(phone, /class="min-h-11 /, 'Steps taps at 44 px on a phone');
  const plain = scene('Quant', { live: { ...live, steps: [{ id: 'r', kind: 'tool', status: 'running', title: 'Using `Read` (notes.md)', started_at: '2026-11-25T00:00:00Z' }] } });
  assert.match(plain, /data-glyph="lines"/);
});

test('the bot is the real creature, without its ground shadow, and the paw wears its body colour', () => {
  const html = scene('Builder');
  assert.ok(body(html), 'the creature body is drawn');
  assert.equal(paw(html), body(html));
  assert.doesNotMatch(html, /fill="#261d4a"/, 'the portrait shadow is dropped');
  assert.match(html, /<g transform="translate\(-5 -2\)"><g class="kw-wa-bob">/, 'translate outside, bob inside');
});

test('a different speaker draws a different bot (team discussions follow the current speaker)', () => {
  const names = ['Builder', 'Writer', 'Designer', 'Director', 'Scout', 'Atlas', 'Quill', 'Echo'];
  const bodies = new Set(names.map((n) => body(scene(n))));
  assert.ok(bodies.size > 1, 'speakers do not all share one colour');
  for (const n of names) assert.equal(paw(scene(n)), body(scene(n)), `${n}: paw = body`);
  assert.match(chat, /<WorkingAvatar\s+name=\{\s+discussionState\?\.inProgress && discussionState\.currentSpeaker\s+\? discussionState\.currentSpeaker\s+: streamingSpeakerName \|\| title\s+\}/);
});

test('36d: writing keeps the scene on top and the words stream under its hairline (no hold, no swap)', () => {
  assert.doesNotMatch(chat, /kws-skel|Waiting for the first words/);
  assert.match(chat, /const shownWords = \(text: string\): string => hideStreamingDecisions\(stripDiscussionMarkers\(text\)\);/);
  assert.match(chat, /return <StreamMarkdown content=\{shownWords\(streamingText\)\} \/>;/);
  assert.doesNotMatch(chat + css, /kw-wa-hold \{|className="[^"]*kw-wa-hold/);
  assert.match(chat, /\{writing \? <StreamPart stream=\{stream\} part="text" onGrow=\{onStreamGrow\} \/> : null\}\s+<\/WorkingAvatar>/);
  const html = scene('Writer', { writing: true, children: 'Hello' });
  assert.match(html, /is writing the reply/);
  assert.match(html, /<div class="kw-wa-words border-t border-hairline" data-scene-words="">Hello<\/div><\/div>$/);
  assert.match(css, /\.kw-wa-stage \{[^}]*width: 120px;[^}]*height: 64px;[^}]*background: var\(--kw-surface-main\);[^}]*box-shadow: inset 0 0 0 1px var\(--kw-hairline\);/);
  assert.match(css, /\.kw-wa-scene \{[^}]*flex: none;[^}]*height: 54px;/);
  assert.match(css, /\.kw-wa-card\[data-phone\] \.kw-wa-stage \{[^}]*width: 84px; height: 46px;/);
});

test('queued and paused bubbles keep their text; the phone shows the scene, not the paused bubble', () => {
  for (const s of [
    'Paused on the browser step above. Do it in the browser panel and press Done; this reply carries on from it.',
    'Paused on the question above. Answer it there; this reply carries on from it.',
    'Paused on the approval below. Allow or deny it there; this reply carries on from it.',
  ]) assert.ok(chat.includes(s), s);
  assert.match(chat, /const sceneShown = bubble === 'starting' \|\| \(bubble === 'text' && !liveActivity\?\.steps\.some\(\(st\) => st\.status === 'waiting'\)\);/);
  assert.match(chat, /\{sceneShown \? \(/);
  assert.match(chat, /\) : \(!phone \|\| queued \|\| writing\) && \(/, 'phones keep the strip for a wait, no paused bubble');
});

test('motion is CSS keyframes on transform and opacity only, no JS timers, a 2.4 s loop', () => {
  assert.doesNotMatch(tsx, /setTimeout|setInterval|requestAnimationFrame|useEffect|canvas/);
  // 36d: the glyph reuses the scene's keyframes; nothing new moves.
  assert.doesNotMatch(css, /@keyframes kw-wa-(?:cmd|glyph|chev)/);
  const frames = [...css.matchAll(/@keyframes (kw-wa-[\w-]+) \{([\s\S]*?)\n?\}\n/g)];
  assert.ok(frames.length >= 8, `found ${frames.length} keyframes`);
  for (const [, name, b] of frames) {
    const props = [...b.matchAll(/([a-z-]+)\s*:/g)].map((m) => m[1]).filter((p) => p !== 'animation-timing-function');
    for (const p of props) assert.ok(p === 'transform' || p === 'opacity', `${name} animates ${p}`);
  }
  for (const m of css.matchAll(/\.kw-wa-(?:bob|l1|l2|l3|paw) \{ animation: [\w-]+ ([\d.]+)s/g)) assert.ok(Number(m[1]) <= 2.4, m[0]);
});

test('reduced motion stops every animation and leaves the still frame (lines shown, spark hidden, no fade)', () => {
  const rm = css.match(/@media \(prefers-reduced-motion: reduce\) \{\s+\.kw-wa-in, \.kw-live-in, \.kw-wa-scene \* \{ animation: none !important; \}\s+\}/);
  assert.ok(rm, 'reduced-motion block covers the fades (32e: the bubble too) and every scene part');
  assert.doesNotMatch(css, /\.kw-wa \{[^}]*animation/, 'the scene row itself never moves');
  assert.match(css, /\.kw-wa-spark rect \{ opacity: 0; \}/);
  const html = scene('Writer');
  for (const w of [11, 9, 6]) assert.match(html, new RegExp(`class="kw-wa-l kw-wa-l\\d" x="19" y="\\d+" width="${w}"`), `line of ${w} cells drawn at rest`);
});

test('light theme tokens exist for every dark token', () => {
  const dark = css.match(/:root \{([^}]*--kw-wa-[^}]*)\}/)[1];
  const light = css.match(/\[data-theme="light"\] \{([^}]*--kw-wa-[^}]*)\}/)[1];
  const keys = (b) => [...b.matchAll(/(--kw-wa-[\w-]+):/g)].map((m) => m[1]);
  for (const k of ['--kw-wa-ground', '--kw-wa-screen', '--kw-wa-deck', '--kw-wa-edge', '--kw-wa-spark', '--kw-wa-spark-core', '--kw-wa-text', '--kw-wa-name']) {
    assert.ok(keys(dark).includes(k) && keys(light).includes(k), k);
  }
});

test('36d design-check fixes: even dots, steady phone card, bright name in C, 13 px phone row', () => {
  const live = { turnId: 't1', thinking: {}, steps: [{ id: 's1', kind: 'tool', status: 'ok', title: 'Reading `a.md`', started_at: '2026-11-25T00:00:00Z' }] };
  const html = scene('Builder', { live, audit: { line: 'Builder is double-checking Scout’s work', short: 'Double-checking', onOpenTask: () => {} } });
  assert.match(html, /class="kw-wa-meta flex min-w-0 items-center gap-x-2 /);
  assert.match(html, /data-scene-steps="" class="min-h-7 -mx-2 /);
  assert.match(html, /data-scene-line=""><span class="font-medium text-zinc-50">Builder<\/span> is double-checking Scout’s work<\/p>/);
  assert.match(scene('Builder', { live, phone: true, audit: { line: 'Builder is double-checking Scout’s work', short: 'Double-checking' } }), /data-scene-line="">Double-checking<\/p>/);
  assert.doesNotMatch(css, /\[data-scene-steps\] \{ margin-left: 0; \}/);
  assert.match(css, /\.kw-wa-card\[data-phone\] \.kw-wa-meta \{ min-height: 44px; \}/);
  assert.match(css, /\.kw-wa-card\[data-phone\] \.kw-wa-open,\n\.kw-wa-card\[data-phone\] \.kw-wa-meta \[data-watch-button\] \{ font-size: 13px;/);
});
