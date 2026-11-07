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

export const SHOP_SUGGESTION: Suggestion = normSuggestion(SHOP_WIRE, '~/projects/shop');
export const PARTIAL_SUGGESTION: Suggestion = normSuggestion(PARTIAL_WIRE, '~/projects/shop');
export const UNKNOWN_SUGGESTION: Suggestion = normSuggestion(UNKNOWN_WIRE, '~/projects/notes');
export const MANAGE_SUGGESTION: Suggestion = normSuggestion(MANAGE_WIRE, '~/projects/shop');

export type SuggestScenario = 'shop' | 'partial' | 'partial-file' | 'leads' | 'unknown' | 'cantread' | 'error' | 'offline' | 'slow' | 'manage' | 'expired';
export type Call = { url: string; method: string; body?: unknown; headers?: Record<string, string> };

/** A transport answering the T and D routes from fixtures. `calls` collects every request. */
export function fixtureTransport(
  opts: { suggest?: SuggestScenario; create?: 'ok' | 'fail' | 'clash' | 'clash-handle' | 'clash-many' | 'expired-once'; preview?: TemplatePreview | 'error'; templates?: 'ok' | 'error' | 'slow' } = {},
  calls: Call[] = [],
): TeamsTransport {
  let expired = opts.create === 'expired-once';
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
