// 35a: Mistral as a named engine, API only. The family, picks and accounts read like the server's
// (profiles::Engine::from_model, router::family_id), Ollama's own "mistral" stays local, and the
// badge draws a plain letter mark of kwant's own, not Mistral's logo.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {} };

const m = await loadBundled(`
  export * from './src/engines';
  export { engineOfModel, newAccountFor, cardOf, ENGINES, isMistralModel } from './src/profiles';
  export { ENGINE_NAMES } from './src/failover';
  export { EngineMark, MARK_NAME } from './src/components/EngineMark';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export { createElement } from 'react';
`);

test('35a: Mistral is a family of its own with one engine, the API key', () => {
  assert.equal(m.FAMILY_OF.mistral_api, 'mistral');
  assert.equal(m.familyLabel('mistral'), 'Mistral');
  assert.ok(m.ENGINES.some((e) => e.id === 'mistral' && e.label === 'Mistral'));
  assert.equal(m.ENGINE_NAMES.mistral, 'Mistral');
  assert.equal(m.INSTALL_URL.mistral_api, undefined, 'nothing to install: a key is added on its card');
});

test("35a: Mistral's models pick the Mistral family; Ollama's mistral stays on Ollama", () => {
  assert.deepEqual(m.parsePick('mistral'), { family: 'mistral', model: null });
  assert.deepEqual(m.parsePick('mistral/mistral-large-latest'), { family: 'mistral', model: 'mistral-large-latest' });
  for (const id of ['mistral-small-latest', 'devstral-medium-2507', 'magistral-medium-latest', 'codestral-latest', 'ministral-8b-latest']) {
    assert.equal(m.parsePick(id).family, 'mistral', id);
    assert.equal(m.engineOfModel(id), 'mistral', id);
  }
  assert.equal(m.parsePick('ollama/mistral').family, 'ollama');
  assert.equal(m.engineOfModel('ollama/mistral'), null);
  assert.equal(m.engineOfModel('local_ollama'), null);
  // The others are unchanged
  assert.equal(m.engineOfModel('gpt-5'), 'codex');
  assert.equal(m.engineOfModel('claude-opus-5-5'), 'claude');
  assert.equal(m.engineOfModel('grok-4.6'), 'grok');
  assert.equal(m.isMistralModel('gpt-5'), false);
});

test('35a: a Mistral account is an API key on the Mistral API card', () => {
  assert.deepEqual(m.newAccountFor('mistral_api'), { engine: 'mistral', kind: 'api_key' });
  assert.equal(m.cardOf({ engine: 'mistral', kind: 'api_key' }), 'mistral_api');
  assert.equal(m.cardOf({ engine: 'mistral', kind: 'cli_home' }), 'mistral_api', 'no Mistral CLI card');
  assert.equal(m.newAccountFor('mistral'), null, 'no CLI card to sign in on');
});

test("35a: the badge draws a plain letter M named Mistral, not Mistral's logo", () => {
  assert.equal(m.MARK_NAME.mistral, 'Mistral');
  const svg = m.renderToStaticMarkup(m.createElement(m.EngineMark, { mark: 'mistral', size: 14 }));
  assert.match(svg, /data-mark="mistral"/);
  assert.match(svg, /fill="currentColor"/, 'monochrome like the other marks');
  const d = svg.match(/ d="([^"]+)"/)[1];
  assert.ok(d.length < 80, 'a simple letter shape, not a traced logo');
});
