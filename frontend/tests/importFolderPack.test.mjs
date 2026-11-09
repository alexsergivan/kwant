// A picked folder is zipped and sent to the server, which may be remote (Ada's review of #455, "Note for
// Pixel"; release-080/ada-review-455.md). Only files an import format reads are packed, hidden folders
// below the picked one are skipped, and key stores never leave the computer.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const m = await loadBundled(`export { folderPlan, folderZip, leftOutKind } from './src/importSources';`);

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
