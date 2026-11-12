// Example answers for the T and D routes in api.md's wire format, from Mira's d-ui.js (acme/shop).
// Every string is the kind kwant builds itself: framework names from its own list and folder names.
// Tests and shots only.
import type { Suggestion, TeamsTransport, TemplatePreview, WireSuggestion } from './teamTemplates';
import { TeamsError, normSuggestion } from './teamTemplates';
import { TEMPLATES_WIRE } from './templateFixtures';

type Row = WireSuggestion['bots'][number];
type Parts = NonNullable<Row['reason_parts']>;

const bot = (key: string, preset: string, name: string, title: string, sprite: string, reason_parts: Parts, owns: string[] = [], extra: Partial<Row> = {}): Row => ({
  key,
  preset,
  lead: false,
  name,
  title,
  avatar_sprite: sprite,
  reason_parts,
  owns,
  existing: null,
  ...extra,
});
const maybe = (key: string, name: string, title: string, sprite: string, reason: string): Row => ({
  key,
  preset: key,
  lead: false,
  name,
  title,
  avatar_sprite: sprite,
  reason,
  existing: null,
});

export const SHOP_WIRE: WireSuggestion = {
  suggestion_id: 'sug_demo_shop',
  handle: 'shop',
  facts: [
    { name: 'Rust', detail: 'axum 0.7' },
    { name: 'React 18', detail: 'TypeScript · Vite' },
    { name: 'Playwright', detail: '' },
    { name: 'Postgres', detail: '38 migrations' },
    { name: 'Docker', detail: 'compose' },
    { name: 'GitLab CI', detail: '' },
  ],
  bots: [
    bot('lead', 'backend', 'TechLead', 'Tech Lead · Backend', 'architect', [{ text: 'Found axum 0.7 in ' }, { path: 'server/Cargo.toml' }, { text: ' and 186 .rs files under ' }, { path: 'server/' }, { text: '.' }], ['server/'], { lead: true }),
    bot('frontend', 'frontend', 'Frontend', 'Frontend Engineer', 'tech_lead', [{ text: 'Found 412 .tsx files and react 18 in ' }, { path: 'web/package.json' }, { text: '.' }], ['web/'], {
      existing: { kwant_id: 'kw_frontend', name: 'Frontend', avatar_sprite: 'tech_lead', leads_team: null },
    }),
    bot('qa', 'qa', 'QA', 'QA & Test Engineer', 'qa', [{ text: 'Found playwright in ' }, { path: 'web/package.json' }, { text: ' and 64 test files in ' }, { path: 'server/tests/' }, { text: ' and ' }, { path: 'web/e2e/' }, { text: '.' }], ['server/tests/', 'web/e2e/']),
    bot('sre', 'sre', 'SRE', 'Site Reliability Engineer', 'devops', [{ text: 'Found a Dockerfile, ' }, { path: 'docker-compose.yml' }, { text: ' and ' }, { path: '.gitlab-ci.yml' }, { text: '.' }], ['deploy/']),
  ],
  also_possible: [
    maybe('data', 'DataAnalyst', 'Data Analyst', 'data', 'Found 38 .sql files in server/migrations/. Migrations alone are a weak sign.'),
    maybe('security', 'SecurityAuditor', 'Security Auditor', 'security', 'Found async-stripe and jsonwebtoken in server/Cargo.toml.'),
    maybe('writer', 'TechWriter', 'Technical Writer', 'writer', 'Only 6 pages in docs/ (10 make a Tech Writer).'),
  ],
  read: { files: 1240, manifests: 6, ms: 1400 },
  partial: null,
  stack_line: 'This team’s code: Rust (axum), React 18 + TypeScript (Vite), Postgres, Docker',
  fallback: false,
};

export const PARTIAL_WIRE: WireSuggestion = { ...SHOP_WIRE, suggestion_id: 'sug_demo_partial', partial: 'paths', read: { files: 20000, manifests: 9, ms: 6200 } };
export const PARTIAL_FILE_WIRE: WireSuggestion = { ...SHOP_WIRE, suggestion_id: 'sug_demo_partial_file', partial: 'file' };
export const LEADS_WIRE: WireSuggestion = {
  ...SHOP_WIRE,
  suggestion_id: 'sug_demo_leads',
  bots: SHOP_WIRE.bots.map((b) => (b.existing ? { ...b, existing: { ...b.existing, leads_team: 'web' } } : b)),
};

const coding = TEMPLATES_WIRE.find((t) => t.id === 'coding')!;
export const UNKNOWN_WIRE: WireSuggestion = {
  suggestion_id: 'sug_demo_unknown',
  handle: 'notes',
  facts: [],
  bots: coding.bots.map((b) => bot(b.lead ? 'lead' : b.preset, b.preset, b.name, b.title, b.avatar_sprite, [{ text: 'From the Coding template.' }], [], { lead: b.lead })),
  also_possible: [],
  read: { files: 3, manifests: 0, ms: 80 },
  partial: null,
  stack_line: null,
  fallback: true,
};

/** Manage team for @shop: only roles it doesn't have yet. */
export const MANAGE_WIRE: WireSuggestion = {
  ...SHOP_WIRE,
  suggestion_id: 'sug_demo_manage',
  bots: [SHOP_WIRE.bots[2], SHOP_WIRE.bots[3]],
  also_possible: [SHOP_WIRE.also_possible[1]],
};

// Team from your own words: the server contract's answer to `{ need }` (brief.md, "Server contract").
// `facts` empty, `read` zeros, `partial` and `stack_line` null, `fallback` false; rows carry `owns`
// ("Takes care of…"), `reason` (tied to the words) and `instructions` (the drafted soul). Example data.
export const BAKERY_NEED = 'I run a small bakery: answer customer emails, post on Instagram, keep track of orders';
const needRow = (key: string, preset: string, name: string, title: string, sprite: string, reason: string, owns: string[], instructions: string, lead = false): Row => ({
  key,
  preset,
  lead,
  name,
  title,
  avatar_sprite: sprite,
  reason,
  owns,
  instructions,
  existing: null,
});
export const BAKERY_WIRE: WireSuggestion = {
  suggestion_id: 'sug_demo_bakery',
  handle: 'bakery',
  need: BAKERY_NEED,
  facts: [],
  bots: [
    needRow(
      'lead',
      'custom',
      'ShopLead',
      'Bakery Lead',
      'product',
      'You run the bakery day to day, so one bot keeps the others on track and asks you before anything goes out.',
      ['the day’s plan', 'questions for you'],
      'You lead a small bakery’s team.\n\nEach morning, list what the team has to do today: emails waiting for an answer, posts to schedule and orders due.\nHand each job to the right teammate and check it is done.\nAsk the owner before anything is sent, posted or promised to a customer.\nKeep your notes short and plain.',
      true,
    ),
    needRow(
      'inbox',
      'inbox',
      'CustomerMail',
      'Customer Email',
      'writer',
      'You asked for customer emails to be answered.',
      ['customer emails', 'reply drafts'],
      'You answer the bakery’s customer emails.\n\nRead new emails and sort them: orders, questions, complaints, other.\nDraft a friendly, short reply for each. Never send without the owner’s OK.\nWhen an email is an order, pass it to OrderKeeper with the date, items and name.',
    ),
    needRow(
      'social',
      'custom',
      'InstaPoster',
      'Instagram Poster',
      'designer',
      'You asked for posts on Instagram.',
      ['Instagram posts', 'captions'],
      'You plan the bakery’s Instagram posts.\n\nSuggest three posts a week: what is fresh, a behind-the-scenes moment, and a customer favourite.\nWrite the caption and say which photo to take.\nNever post without the owner’s OK.',
    ),
    needRow(
      'orders',
      'custom',
      'OrderKeeper',
      'Order Keeper',
      'data',
      'You asked to keep track of orders.',
      ['the order list', 'pickup dates'],
      'You keep the bakery’s order list.\n\nAdd each order with the customer’s name, items, pickup date and whether it is paid.\nEach evening, list tomorrow’s pickups for the owner.\nFlag any order with a missing date or item.',
    ),
  ],
  also_possible: [],
  read: { files: 0, manifests: 0, ms: 0 },
  partial: null,
  stack_line: null,
  fallback: false,
};
/** Try again: the same words, a second draft (fewer bots, other names). */
export const BAKERY_AGAIN_WIRE: WireSuggestion = {
  ...BAKERY_WIRE,
  suggestion_id: 'sug_demo_bakery_2',
  bots: [
    { ...BAKERY_WIRE.bots[0], name: 'Baker', title: 'Head of the Bakery Team' },
    { ...BAKERY_WIRE.bots[1], name: 'MailHelper' },
    { ...BAKERY_WIRE.bots[3], name: 'Orders' },
  ],
};
export const BAKERY_SUGGESTION: Suggestion = normSuggestion(BAKERY_WIRE, 'need');

export type NeedScenario = 'bakery' | 'again' | 'slow' | 'draft_failed' | 'no_ready_account' | 'limit' | 'busy' | 'offline' | 'too_short';

export const SHOP_SUGGESTION: Suggestion = normSuggestion(SHOP_WIRE, '~/projects/shop');
export const PARTIAL_SUGGESTION: Suggestion = normSuggestion(PARTIAL_WIRE, '~/projects/shop');
export const UNKNOWN_SUGGESTION: Suggestion = normSuggestion(UNKNOWN_WIRE, '~/projects/notes');
export const MANAGE_SUGGESTION: Suggestion = normSuggestion(MANAGE_WIRE, '~/projects/shop');

export type SuggestScenario = 'shop' | 'partial' | 'partial-file' | 'leads' | 'unknown' | 'cantread' | 'error' | 'offline' | 'slow' | 'manage' | 'expired';
export type Call = { url: string; method: string; body?: unknown; headers?: Record<string, string> };

/** A transport answering the T and D routes from fixtures. `calls` collects every request. */
export function fixtureTransport(
  opts: { suggest?: SuggestScenario; need?: NeedScenario; create?: 'ok' | 'fail' | 'clash' | 'clash-handle' | 'clash-many' | 'expired-once'; preview?: TemplatePreview | 'error'; templates?: 'ok' | 'error' | 'slow' } = {},
  calls: Call[] = [],
): TeamsTransport {
  let expired = opts.create === 'expired-once';
  let needCalls = 0;
  return async (url, method, body, headers) => {
    calls.push({ url, method, body, headers });
    const path = url.split('?')[0];
    if (path === '/api/team-templates') {
      if (opts.templates === 'error') return { status: 500, body: { error: 'boom' } };
      if (opts.templates === 'slow') await new Promise((r) => setTimeout(r, 60_000));
      return { status: 200, body: { templates: TEMPLATES_WIRE } };
    }
    const pv = /^\/api\/team-templates\/([^/]+)\/preview$/.exec(path);
    if (pv) {
      // Optional route (§0.10): by default answer 404 like a server without it, so the client's own guess stands.
      if (!opts.preview || opts.preview === 'error') return { status: 404, body: { error: 'not found' } };
      return { status: 200, body: opts.preview };
    }
    if (path === '/api/teams/suggest' && body && typeof (body as { need?: unknown }).need === 'string') {
      // `again`: the first ask gives the bakery team, every later one the second draft.
      const sc = opts.need ?? 'bakery';
      needCalls++;
      if (sc === 'offline') throw new TeamsError('Couldn’t reach the server.', 0, 'offline');
      // unref: a test that cancels the ask doesn't wait the minute out (no-op in a browser).
      if (sc === 'slow') await new Promise((r) => (setTimeout(r, 60_000) as unknown as { unref?: () => void }).unref?.());
      if (sc === 'draft_failed') return { status: 502, body: { error: 'The draft came back unusable twice.', code: 'draft_failed' } };
      if (sc === 'no_ready_account') return { status: 409, body: { error: 'No account in this space can run right now.', code: 'no_ready_account' } };
      if (sc === 'limit') return { status: 429, body: { error: 'Claude is out until 21:10.', code: 'limited', account: 'Claude', until: '2026-11-12T21:10:00Z' } };
      if (sc === 'busy') return { status: 409, body: { error: 'Already drafting a team for this space.', code: 'busy' } };
      if (sc === 'too_short') return { status: 422, body: { error: 'need is too short', code: 'need_too_short' } };
      const need = (body as { need: string }).need;
      return { status: 200, body: { ...(sc === 'again' && needCalls > 1 ? BAKERY_AGAIN_WIRE : BAKERY_WIRE), need } };
    }
    if (path === '/api/teams/suggest') {
      const sc = opts.suggest ?? 'shop';
      if (sc === 'offline') throw new TeamsError('Couldn’t reach the server.', 0, 'offline');
      if (sc === 'error') return { status: 502, body: { error: 'The read stopped.', code: 'read_failed' } };
      if (sc === 'cantread') return { status: 422, body: { error: 'Can’t read this repo.', code: 'cant_read', forge: 'gitlab' } };
      if (sc === 'slow') await new Promise((r) => setTimeout(r, 60_000));
      const s = sc === 'partial' ? PARTIAL_WIRE : sc === 'partial-file' ? PARTIAL_FILE_WIRE : sc === 'leads' ? LEADS_WIRE : sc === 'unknown' ? UNKNOWN_WIRE : sc === 'manage' ? MANAGE_WIRE : SHOP_WIRE;
      return { status: 200, body: s };
    }
    if (path === '/api/teams/from-template') {
      if (opts.create === 'fail') return { status: 500, body: { error: 'stopped' } };
      if (opts.create === 'clash')
        return { status: 409, body: { error: 'That name is taken.', code: 'name_taken', field: 'names.Drafter', suggestion: 'Drafter3', clashes: [{ field: 'names.Drafter', wanted: 'Drafter', suggestion: 'Drafter3' }] } };
      if (opts.create === 'clash-handle')
        return { status: 409, body: { error: 'That handle is taken.', code: 'handle_taken', field: 'handle', suggestion: 'inbox3', clashes: [{ field: 'handle', wanted: 'inbox', suggestion: 'inbox3' }] } };
      if (opts.create === 'clash-many')
        return {
          status: 409,
          body: {
            error: 'That handle is taken.',
            code: 'handle_taken',
            field: 'handle',
            suggestion: 'inbox3',
            clashes: [
              { field: 'handle', wanted: 'inbox', suggestion: 'inbox3' },
              { field: 'names.Drafter', wanted: 'Drafter', suggestion: 'Drafter3' },
              { field: 'names.Researcher', wanted: 'Researcher', suggestion: 'Researcher2' },
            ],
          },
        };
      if (expired) {
        expired = false;
        return { status: 410, body: { error: 'suggestion expired', code: 'suggestion_expired' } };
      }
      const b = body as { template_id?: string; handle?: string; names?: Record<string, string>; edits?: { handle?: string } };
      const handle = b.handle ?? b.edits?.handle ?? 'team';
      const t = TEMPLATES_WIRE.find((x) => x.id === b.template_id);
      const sid = (body as { suggestion_id?: string }).suggestion_id;
      const needWire = [BAKERY_WIRE, BAKERY_AGAIN_WIRE].find((w) => w.suggestion_id === sid);
      if (needWire) {
        const rows = (b.edits as { rows?: Record<string, { on?: boolean; name?: string }> } | undefined)?.rows ?? {};
        const on = needWire.bots.filter((x) => rows[x.key]?.on !== false);
        return {
          status: 201,
          body: {
            team: { id: 'team_new', name: handle },
            channel_id: 'ch_team_new',
            action_id: 'act_demo',
            template_id: null,
            bots: on.map((x, i) => ({ slot: x.key, id: `kw_${i}`, name: rows[x.key]?.name ?? x.name, preset: x.preset, lead: x.lead, existing: false })),
            skills: [],
            routine: null,
            undo_until: '2026-11-13T20:00:00Z',
          },
        };
      }
      return {
        status: 201,
        body: {
          team: { id: 'team_new', name: handle },
          channel_id: 'ch_team_new',
          action_id: 'act_demo',
          template_id: b.template_id ?? null,
          bots: (t?.bots ?? []).map((x, i) => ({ slot: x.slot, id: `kw_${i}`, name: (x.slot && b.names?.[x.slot]) || x.name, preset: x.preset, lead: x.lead, existing: false })),
          skills: (t?.skills ?? []).map((s, i) => ({ id: `sk_${i}`, name: s.name, owner_id: 'kw_0' })),
          routine: t?.routine ? { id: 'rt_0', name: t.routine.name, enabled: false } : null,
          undo_until: '2026-11-08T10:00:00Z',
        },
      };
    }
    return { status: 404, body: { error: 'not found' } };
  };
}
