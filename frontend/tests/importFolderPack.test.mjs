// A picked folder is zipped and sent to the server, which may be remote (Ada's review of #455, "Note for
// Pixel"; release-080/ada-review-455.md). Only files an import format reads are packed, hidden folders
// below the picked one are skipped, and key stores never leave the computer.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const m = await loadBundled(`export { folderPlan, folderZip, leftOutKind, appHomes } from './src/importSources';`);

const pick = (files) => files.map(([path, body = 'x']) => ({ path, size: body.length, file: new Blob([body]) }));
const kept = (plan) => plan.kept.map((e) => e.path).sort();
const zipNames = async (plan) => {
  const { body } = await m.folderZip(plan);
  const bytes = new Uint8Array(await body.arrayBuffer());
  // Central directory: signature 0x02014b50, name length at +28, name at +46.
  const names = [];
  for (let i = 0; i + 46 <= bytes.length; i++) {
    if (bytes[i] === 0x50 && bytes[i + 1] === 0x4b && bytes[i + 2] === 0x01 && bytes[i + 3] === 0x02) {
      const n = bytes[i + 28] | (bytes[i + 29] << 8);
      names.push(new TextDecoder().decode(bytes.slice(i + 46, i + 46 + n)));
    }
  }
  return { names, text: new TextDecoder('latin1').decode(bytes) };
};
const KEY = 'sk-live-DO-NOT-SEND-1234567890';

test('OpenClaw: openclaw.json, credentials/ and sessions/ are never packed', async () => {
  const plan = m.folderPlan(
    pick([
      ['.openclaw/openclaw.json', `{"apiKey":"${KEY}"}`],
      ['.openclaw/credentials/oauth.json', `{"token":"${KEY}"}`],
      ['.openclaw/agents/main/sessions/s1.json', `{"m":"${KEY}"}`],
      ['.openclaw/agents/main/sessions/s1.md', KEY],
      ['.openclaw/workspace/SOUL.md', 'be kind'],
      ['.openclaw/workspace/memory/2026-11-01.md', 'note'],
      ['.openclaw/workspace/skills/seo/SKILL.md', '# seo'],
      ['.openclaw/workspace/notes.txt', 'random'],
      ['.openclaw/workspace/avatar.png', 'png'],
    ]),
  );
  assert.deepEqual(kept(plan), [
    '.openclaw/workspace/SOUL.md',
    '.openclaw/workspace/memory/2026-11-01.md',
    '.openclaw/workspace/skills/seo/SKILL.md',
  ]);
  const { names, text } = await zipNames(plan);
  assert.ok(!names.some((n) => /openclaw\.json|credentials\/|sessions\//.test(n)), names.join(', '));
  assert.ok(!text.includes(KEY));
  assert.ok(plan.left.some((l) => /never sent/i.test(l.label)), JSON.stringify(plan.left));
});

test('Codex: auth.json is never packed; config.toml goes only as an empty name', async () => {
  const plan = m.folderPlan(
    pick([
      ['.codex/auth.json', `{"OPENAI_API_KEY":"${KEY}"}`],
      ['.codex/config.toml', `api_key = "${KEY}"`],
      ['.codex/AGENTS.md', 'do it'],
      ['.codex/sessions/2026/11/09/rollout.jsonl', KEY],
      ['.codex/skills/fix/SKILL.md', '# fix'],
    ]),
  );
  assert.deepEqual(kept(plan), ['.codex/AGENTS.md', '.codex/skills/fix/SKILL.md']);
  const { names, text } = await zipNames(plan);
  assert.ok(names.includes('.codex/config.toml'), 'Codex marker kept by name');
  assert.ok(!names.some((n) => /auth\.json|sessions\//.test(n)), names.join(', '));
  assert.ok(!text.includes(KEY), 'no key bytes in the zip');
});

test('Hermes: .env, state.db and config.yaml stay; memories and skills go', () => {
  const plan = m.folderPlan(
    pick([
      ['.hermes/.env', `OPENAI_API_KEY=${KEY}`],
      ['.hermes/state.db', KEY],
      ['.hermes/state.db-wal', KEY],
      ['.hermes/config.yaml', `key: ${KEY}`],
      ['.hermes/auth.json', KEY],
      ['.hermes/SOUL.md', 'soul'],
      ['.hermes/memories/MEMORY.md', 'm'],
      ['.hermes/memories/USER.md', 'u'],
      ['.hermes/skills/.bundled_manifest', 'a'],
      ['.hermes/skills/web/search/SKILL.md', 's'],
    ]),
  );
  assert.deepEqual(kept(plan), [
    '.hermes/SOUL.md',
    '.hermes/memories/MEMORY.md',
    '.hermes/memories/USER.md',
    '.hermes/skills/.bundled_manifest',
    '.hermes/skills/web/search/SKILL.md',
  ]);
});

test('hidden folders below the picked one, and project files, are skipped', () => {
  const plan = m.folderPlan(
    pick([
      ['team/.git/config', 'x'],
      ['team/.cache/skills/x/SKILL.md', 'x'],
      ['team/.claude/agents/hidden.md', 'x'],
      ['team/node_modules/a/package.json', '{}'],
      ['team/package.json', '{}'],
      ['team/tsconfig.json', '{}'],
      ['team/README.md', 'read me'],
      ['team/src/index.ts', 'x'],
      ['team/.paperclip.yaml', 'routines: []'],
      ['team/COMPANY.md', 'c'],
      ['team/agents/ceo/AGENTS.md', 'a'],
      ['team/agents/reviewer.md', 'r'],
      ['team/employees/ada/employee.json', '{}'],
      ['team/export.json', '{}'],
      ['team/LICENSE', 'MIT'],
    ]),
  );
  assert.deepEqual(kept(plan), [
    'team/.paperclip.yaml',
    'team/COMPANY.md',
    'team/LICENSE',
    'team/agents/ceo/AGENTS.md',
    'team/agents/reviewer.md',
    'team/employees/ada/employee.json',
    'team/export.json',
  ]);
});

test('key-store names are refused at any depth and in any case', () => {
  for (const p of ['x/OpenClaw.json', 'x/a/AUTH.json', 'x/Credentials/a.md', 'x/a/Sessions/b.md', 'x/.env.local', 'x/a/state.db'])
    assert.notEqual(m.leftOutKind(p, 1), null, p);
});

// Ada's review of #459, must-fix 1: every key-bearing file her probe saw packed.
const PROBE = 'sk-ant-PROBE';
const KEY_FILES = [
  '.openclaw/openclaw.json',
  '.openclaw/agents/main/agent/auth-profiles.json',
  '.openclaw/agents/main/agent/models.json',
  '.openclaw/identity/device.json',
  '.openclaw/devices/paired.json',
  '.openclaw/cron/jobs.json',
  '.openclaw/.claude/settings.json',
  '.openclaw/.claude/settings.local.json',
  '.openclaw/credentials.json',
  '.openclaw/token.json',
  '.openclaw/client_secret_123.json',
  '.openclaw/service-account.json',
  '.openclaw/mcp.json',
  '.openclaw/.hermes/gateway_state.json',
  '.openclaw/workspace/settings.json',
  '.openclaw/workspace/oauth-cache.json',
];

test('a picked app folder never packs its key files: only the .md files local import reads', async () => {
  const plan = m.folderPlan(
    pick([
      ...KEY_FILES.map((p) => [p, `{"key":"${PROBE}"}`]),
      ['.openclaw/workspace/SOUL.md', 'be kind'],
      ['.openclaw/workspace/skills/x/SKILL.md', '# x'],
      ['.openclaw/workspace/memory/api-key-rotation.md', 'rotate monthly'],
      ['.openclaw/workspace/export.json', '{"name":"not read in an app home"}'],
    ]),
  );
  assert.deepEqual(kept(plan), [
    '.openclaw/workspace/SOUL.md',
    '.openclaw/workspace/memory/api-key-rotation.md',
    '.openclaw/workspace/skills/x/SKILL.md',
  ]);
  const { names, text } = await zipNames(plan);
  for (const f of KEY_FILES) assert.ok(!names.includes(f), f);
  assert.ok(!names.some((n) => n.endsWith('.json')), names.join(', '));
  assert.ok(!text.includes(PROBE), 'no sk-ant-PROBE bytes in the zip');
  assert.ok(plan.left.some((l) => /never sent/i.test(l.label)), JSON.stringify(plan.left));
});

test('a picked .claude never packs settings.json; its agents and skills go', async () => {
  const plan = m.folderPlan(
    pick([
      ['.claude/settings.json', `{"env":{"ANTHROPIC_API_KEY":"${PROBE}"}}`],
      ['.claude/settings.local.json', `{"env":{"K":"${PROBE}"}}`],
      ['.claude/.credentials.json', PROBE],
      ['.claude/agents/reviewer.md', 'r'],
      ['.claude/skills/oauth-helper/SKILL.md', 's'],
    ]),
  );
  assert.deepEqual(kept(plan), ['.claude/agents/reviewer.md', '.claude/skills/oauth-helper/SKILL.md']);
  assert.ok(!(await zipNames(plan)).text.includes(PROBE));
});

test('a generic folder drops named key files but keeps bot .json (employee.json, conversations.json)', async () => {
  const plan = m.folderPlan(
    pick([
      ['team/credentials.json', PROBE],
      ['team/token.json', PROBE],
      ['team/client_secret_1.json', PROBE],
      ['team/service-account-prod.json', PROBE],
      ['team/my-api_key.txt', PROBE],
      ['team/private.yaml', PROBE],
      ['team/employees/ada/employee.json', '{"name":"Ada"}'],
      ['team/employees/ada/memory/api-key-rotation.md', 'rotate'],
      ['team/conversations.json', '[]'],
    ]),
  );
  assert.deepEqual(kept(plan), [
    'team/conversations.json',
    'team/employees/ada/employee.json',
    'team/employees/ada/memory/api-key-rotation.md',
  ]);
  const { text } = await zipNames(plan);
  assert.ok(!text.includes(PROBE), 'no sk-ant-PROBE bytes in the zip');
  assert.equal(plan.left.find((l) => /never sent/i.test(l.label))?.count, 6, JSON.stringify(plan.left));
});

// ------------------------------------------------------------------ A4 follow-ups 8–10 (Ada's #459 review items 10–12)

test('item 8: a renamed OpenClaw copy (no openclaw.json, not hidden) never sends .json under identity/ or devices/', async () => {
  const plan = m.folderPlan(
    pick([
      ['claw-backup/identity/device.json', `{"privateKey":"${PROBE}"}`],
      ['claw-backup/Identity/keys.json', `{"k":"${PROBE}"}`],
      ['claw-backup/devices/paired.json', `{"token":"${PROBE}"}`],
      ['claw-backup/devices/pending/phone.json', `{"k":"${PROBE}"}`],
      ['claw-backup/workspace/SOUL.md', 'be kind'],
      ['claw-backup/workspace/skills/x/SKILL.md', '# x'],
      ['claw-backup/team/employee.json', '{"name":"Ada"}'],
    ]),
  );
  assert.deepEqual(kept(plan), ['claw-backup/team/employee.json', 'claw-backup/workspace/SOUL.md', 'claw-backup/workspace/skills/x/SKILL.md']);
  const { names, text } = await zipNames(plan);
  assert.ok(!names.some((n) => /identity\/|devices\//i.test(n)), names.join(', '));
  assert.ok(!text.includes(PROBE), 'no key bytes in the zip');
  assert.equal(plan.left.find((l) => /never sent/i.test(l.label))?.count, 4, JSON.stringify(plan.left));
});

test('item 9: .env.example, config.yaml and config.toml alone don’t make a kit an app home; its bot .json still goes', async () => {
  const plan = m.folderPlan(
    pick([
      ['kit/.env.example', `OPENAI_API_KEY=${PROBE}`],
      ['kit/config.yaml', `api_key: ${PROBE}`],
      ['kit/cards/config.toml', `key = "${PROBE}"`],
      ['kit/employees/ada/employee.json', '{"name":"Ada"}'],
      ['kit/cards/molty.json', '{"spec":"chara_card_v2"}'],
      ['kit/skills/seo/SKILL.md', '# seo'],
    ]),
  );
  assert.deepEqual(kept(plan), ['kit/cards/molty.json', 'kit/employees/ada/employee.json', 'kit/skills/seo/SKILL.md']);
  const { names, text } = await zipNames(plan);
  assert.ok(!names.some((n) => /\.env|config\.yaml/.test(n)), names.join(', '));
  assert.ok(!text.includes(PROBE), 'the marker files themselves are never sent');
  assert.deepEqual(m.appHomes(plan.kept.map((e) => e.path).concat(['kit/.env.example', 'kit/config.yaml', 'kit/cards/config.toml'])), []);
});

test('item 9: the same markers inside an app-named folder, or next to a real key store, still mark an app home', async () => {
  const plan = m.folderPlan(
    pick([
      ['backup/openclaw/.env', `K=${PROBE}`],
      ['backup/openclaw/export.json', `{"k":"${PROBE}"}`],
      ['backup/openclaw/SOUL.md', 'be kind'],
      ['backup/hub/auth.json', `{"k":"${PROBE}"}`],
      ['backup/hub/config.yaml', `k: ${PROBE}`],
      ['backup/hub/models.json', `{"k":"${PROBE}"}`],
      ['backup/hub/AGENTS.md', 'work'],
    ]),
  );
  assert.deepEqual(kept(plan), ['backup/hub/AGENTS.md', 'backup/openclaw/SOUL.md']);
  assert.ok(!(await zipNames(plan)).text.includes(PROBE));
  assert.deepEqual(m.appHomes(['backup/openclaw/.env', 'backup/hub/auth.json']).sort(), ['hub', 'openclaw']);
});

test('item 10: "auth" counts only as a whole word part, so authors-note.json and author.json still go', async () => {
  const plan = m.folderPlan(
    pick([
      ['cards/authors-note.json', '{"note":"by Ada"}'],
      ['cards/author.json', '{"name":"Ada"}'],
      ['cards/my-auth.json', `{"k":"${PROBE}"}`],
      ['cards/auth_cache.json', `{"k":"${PROBE}"}`],
      ['cards/github-authz.json', `{"k":"${PROBE}"}`],
      ['cards/AUTH.yaml', `k: ${PROBE}`],
      ['cards/oauthcache.json', `{"k":"${PROBE}"}`],
    ]),
  );
  assert.deepEqual(kept(plan), ['cards/author.json', 'cards/authors-note.json']);
  assert.ok(!(await zipNames(plan)).text.includes(PROBE), 'no key bytes in the zip');
  assert.equal(plan.left.find((l) => /never sent/i.test(l.label))?.count, 5, JSON.stringify(plan.left));
});
