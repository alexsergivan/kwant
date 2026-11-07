// Generates src/teams/templates.json (from Mira's design/templates.js) and
// src/teams/presets.json (from frontend/src/agentPresets.ts). Run:
//   node scripts/gen-team-data.mjs <path to next-077/design/templates.js>
// Both outputs are checked in; src/teams tests check presets.json against agentPresets.ts.
import fs from 'node:fs';
import path from 'node:path';
import vm from 'node:vm';

const root = path.resolve(path.dirname(new URL(import.meta.url).pathname), '..');
const tplSrc = process.argv[2];
if (tplSrc) {
  const ctx = { window: {} };
  vm.runInNewContext(fs.readFileSync(tplSrc, 'utf8'), ctx);
  fs.writeFileSync(path.join(root, 'src/teams/templates.json'), JSON.stringify(ctx.window.TEMPLATES, null, 1) + '\n');
}
const ts = fs.readFileSync(path.join(root, 'frontend/src/agentPresets.ts'), 'utf8');
const start = ts.indexOf('[', ts.indexOf('export const AGENT_PRESETS'));
const end = ts.indexOf('\n];', start);
const presets = vm.runInNewContext('(' + ts.slice(start, end + 2) + ')');
const out = presets.map((p) => ({ id: p.id, name: p.name, title: p.title, avatar_sprite: p.avatar_sprite, persona_prompt: p.persona_prompt, soul: p.soul }));
fs.writeFileSync(path.join(root, 'src/teams/presets.json'), JSON.stringify(out, null, 1) + '\n');
console.log(`presets: ${out.length}`);
