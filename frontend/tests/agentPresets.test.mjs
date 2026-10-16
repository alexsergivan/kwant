import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import ts from 'typescript';

const load = async (path) => {
  const source = await readFile(new URL(path, import.meta.url), 'utf8');
  const { outputText } = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ESNext, jsx: ts.JsxEmit.ReactJSX },
  });
  return outputText;
};

const presets = await import(`data:text/javascript;base64,${Buffer.from(await load('../src/agentPresets.ts')).toString('base64')}`);
const { AGENT_PRESETS, uniqueAgentName } = presets;

test('picks a free name when the preset name is taken', () => {
  assert.equal(uniqueAgentName('QA', ['Architect']), 'QA');
  assert.equal(uniqueAgentName('QA', ['qa', 'QA2']), 'QA3');
});

test('presets have mentionable names, distinct ids and real avatar styles', async () => {
  const avatarSource = await load('../src/components/ModernAvatar.tsx');
  const styleIds = Array.from(avatarSource.matchAll(/\{ id: '([a-z_]+)', label:/g), (m) => m[1]);
  assert.ok(styleIds.length >= 12);
  assert.equal(new Set(AGENT_PRESETS.map((p) => p.id)).size, AGENT_PRESETS.length);
  for (const p of AGENT_PRESETS) {
    assert.match(p.name, /^[^\s@]+$/, p.name);
    assert.ok(styleIds.includes(p.avatar_sprite), `${p.id} uses unknown avatar ${p.avatar_sprite}`);
    assert.ok(p.persona_prompt.length > 100, `${p.id} persona is too thin`);
  }
});
