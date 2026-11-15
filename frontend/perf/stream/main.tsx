// Streaming harness (dev only, never shipped; perf/stream/README.md). The real SpaceApp, with the server
// faked in the page: one channel holding ?n= messages (default 300: markdown, code blocks, a table and a
// chart result), and an event stream the runner drives through window.__stream.
import { FIXTURES } from '../../src/components/results/fixtures';
// The app's own stylesheets, as main.tsx loads them.
import '../../src/styles/fonts.css';
import '../../src/styles/modern.css';
import '../../src/styles/stream.css';
import '../../src/styles/needsYou.css';
import '../../src/styles/ideas.css';
import '../../src/styles/ideaShell.css';
import '../../src/styles/devStatus.css';
import '../../src/styles/tickets.css';
import '../../src/styles/telegram.css';
import '../../src/styles/showOnce.css';
import '../../src/styles/gettingStarted.css';
import '../../src/styles/dropLayer.css';
import { trackAppHeight } from '../../src/pwa';

const q = new URLSearchParams(location.search);
const N = Number(q.get('n') ?? 300);
const CTX = 'ch1';
const T0 = Date.parse('2026-11-14T09:00:00Z');
const at = (i: number) => new Date(T0 + i * 60_000).toISOString();

// ---- 300 messages: alternating person / bot, the bot's replies cycling through shapes ----
const code = (i: number) =>
  `Here is the fix for step ${i}:\n\n\`\`\`ts\nexport function total(rows: Row[]): number {\n  let sum = 0;\n  for (const r of rows) sum += r.amount * (1 + r.tax);\n  return Math.round(sum * 100) / 100;\n}\n\`\`\`\n\nRun \`npm test\` after it.`;
const prose = (i: number) =>
  `**Step ${i}.** I read the notes and the two files you sent. Three things stand out:\n\n1. The *rates cache* is read before it is filled.\n2. The receipts list is sorted twice.\n3. The period ends one day early, see [the spec](https://example.com/spec).\n\n> Keep the old order for exports.\n\nI changed the first two and left the third for you to decide.`;
const list = (i: number) =>
  `Checked ${i} items:\n\n- [x] invoices match\n- [x] totals add up\n- [ ] the March receipt is missing\n\nEverything else is in place.`;
const messages: unknown[] = [];
for (let i = 0; i < N; i++) {
  if (i % 2 === 0) {
    messages.push({ id: `m${i}`, sender_role: 'user', sender_id: 'user', text: `Question ${i}: can you check the next part?`, channel_id: CTX, created_at: at(i) });
    continue;
  }
  let text: string;
  let results: unknown = null;
  if (i === 41) ({ reply: text, results } = FIXTURES.comparison as { reply: string; results: unknown });
  else if (i === 83) ({ reply: text, results } = FIXTURES.chartLine as { reply: string; results: unknown });
  else text = [prose, code, list][Math.floor(i / 2) % 3](i);
  messages.push({ id: `m${i}`, sender_role: 'kwant', sender_id: 'Ada', text, results, channel_id: CTX, created_at: at(i), tokens_used: 400 });
}

// ---- the server ----
const bot = (id: string, name: string, title: string) => ({ id, name, title, persona_prompt: '', avatar_sprite: 'mouse', driver_type: 'claude', preferred_model: '' });
const routes: Record<string, unknown> = {
  '/api/kwants': [bot('k1', 'Ada', 'Tech lead'), bot('k2', 'Lin', 'Backend')],
  '/api/channels': [{ id: CTX, name: 'general', kwant_ids: ['k1', 'k2'], created_at: '' }],
  '/api/teams': [],
  '/api/engines': [{ id: 'claude', kind: 'cli', enabled: true, status: 'ready', ready: true }],
  '/api/automation-discovery': { setting: 'off', daily_cap: 3, blocked_by: [], last_scan_at: null, suggestions: [], stats: {} },
  [`/api/channels/${CTX}/messages`]: messages,
};
const json = (b: unknown) => new Response(JSON.stringify(b), { status: 200, headers: { 'Content-Type': 'application/json' } });
window.fetch = async (url: RequestInfo | URL, init?: RequestInit) => {
  const path = String(url instanceof Request ? url.url : url).replace(location.origin, '').split('?')[0];
  return json(path in routes ? routes[path] : init?.method === 'PUT' || init?.method === 'POST' ? {} : []);
};
const streams: FakeEvents[] = [];
class FakeEvents {
  onmessage: ((e: { data: string }) => void) | null = null;
  onerror: (() => void) | null = null;
  onopen: (() => void) | null = null;
  readyState = 1;
  constructor() {
    streams.push(this);
  }
  addEventListener() {}
  removeEventListener() {}
  close() {
    this.readyState = 2;
  }
}
(window as unknown as { EventSource: unknown }).EventSource = FakeEvents;
const emit = (ev: Record<string, unknown>) => streams.forEach((s) => s.readyState === 1 && s.onmessage?.({ data: JSON.stringify(ev) }));

// ---- the reply that streams: ~2,000 tokens of mixed markdown, cut into 1-6 character tokens ----
const replyText = (() => {
  let s = '';
  for (let i = 0; s.length < 9000; i++) s += `${[prose, code, list][i % 3](1000 + i)}\n\n`;
  return s;
})();
const tokens: string[] = [];
for (let i = 0, k = 0; i < replyText.length; k++) {
  const len = 1 + ((k * 7) % 6);
  tokens.push(replyText.slice(i, i + len));
  i += len;
}

declare global {
  interface Window {
    __harness: { ready: () => boolean; stream: (count: number, perSecond: number) => Promise<{ sent: number; ms: number }>; tokens: number; streamed: () => string };
  }
}
let streamed = '';
window.__harness = {
  tokens: tokens.length,
  ready: () => document.querySelectorAll('[data-message-id]').length >= Math.min(N, 20),
  streamed: () => streamed,
  // Sends `count` tokens at `perSecond`, on a timer, the way the server's events arrive.
  stream: (count, perSecond) =>
    new Promise((done) => {
      emit({ type: 'turn_started', kwant_id: 'k1', context_id: CTX, data: { turn_id: 't1', kwant_id: 'k1', context_id: CTX } });
      const t0 = performance.now();
      let sent = 0;
      const tick = () => {
        // Catch up when a long frame held the timer back, as the event stream would.
        const due = Math.min(count, Math.floor(((performance.now() - t0) / 1000) * perSecond) + 1);
        while (sent < due) {
          const tok = tokens[sent % tokens.length];
          streamed += tok;
          emit({ type: 'TextDelta', kwant_id: 'k1', context_id: CTX, turn_id: 't1', data: tok });
          sent++;
        }
        if (sent >= count) return done({ sent, ms: performance.now() - t0 });
        setTimeout(tick, 1000 / perSecond);
      };
      tick();
    }),
};

const [{ default: React }, { createRoot }, { SpaceApp }] = await Promise.all([import('react'), import('react-dom/client'), import('../../src/App')]);
const space = { id: 'spc_default', name: 'Default', icon: '', color: '#f59e0b', sort: 0, is_default: true, archived_at: null, created_at: '', general_channel_id: CTX, counts: { bots: 2, ready_accounts: 1, unread: 0 } };
const noop = () => {};
trackAppHeight();
// Shell's frame (Shell.tsx), so the chat scrolls inside the window the way it does in the app.
const frame = (child: React.ReactNode) =>
  React.createElement(
    'div',
    { className: 'h-[var(--kwnt-app-height,100dvh)] w-screen flex flex-col bg-zinc-950 text-zinc-100 overflow-hidden font-sans' },
    React.createElement('div', { className: 'flex-1 min-h-0 flex' }, child),
  );
createRoot(document.getElementById('root')!).render(
  frame(React.createElement(SpaceApp as React.FC<Record<string, unknown>>, {
    space,
    spaces: [space],
    initialTarget: { channelId: CTX },
    onSwitchSpace: noop,
    onCreateSpace: noop,
    onOpenArchived: noop,
    onSpacesChanged: noop,
    onSpaceArchived: noop,
    onChatChange: noop,
  })),
);
