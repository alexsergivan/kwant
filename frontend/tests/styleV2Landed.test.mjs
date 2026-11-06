// Dev mode style v2 (Mira, style-v2.md §2 and §5): the shared parts (LED, diffstat, sha chip), the landed
// card's chips, diffstats and tests line, and the git rail on What landed (IR-2, IR-4). Demo data only.
process.env.TZ = 'UTC';

import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { loadBundled } from './bundle.mjs';

const m = await loadBundled(`
  export * from './src/dev/types';
  export * from './src/dev/logic';
  export * as fx from './src/dev/fixtures';
  export { Led, DiffStat, Sha } from './src/components/dev/parts';
  export { LandedCard, LandedList } from './src/components/dev/LandedCard';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export { createElement } from 'react';
`);
const { fx } = m;
const html = (C, props) => m.renderToStaticMarkup(m.createElement(C, props));
const landed = (raw, over = {}) => ({ ...m.parseLanded(raw), ...over });
const NOW = fx.FIXTURE_NOW;

test('IR-2 chips row: area · sha chip · #pr (mono link) · Verified; a direct push says so', () => {
  const pr = html(m.LandedCard, { card: landed(fx.LANDED_289) });
  const row = pr.slice(0, pr.indexOf('<h3'));
  assert.match(row, /class="dv-sha"/);
  assert.match(row, /aria-label="PR #289"[^>]*font-mono[^>]*>#289<\/a>/);
  assert.match(row, />Verified</);
  assert.equal([...row.matchAll(/kw-sticker/g)].length > 0, true);
  assert.doesNotMatch(row, /Direct push/);
  const direct = html(m.LandedCard, { card: landed(fx.LANDED_DIRECT) });
  assert.match(direct.slice(0, direct.indexOf('<h3')), /c0ffee1<\/span><span class="kw-chip">Direct push<\/span>/);
});

test('IR-2 under the hood: total diffstat on the summary and one per file row', () => {
  const out = html(m.LandedCard, { card: landed(fx.LANDED_288) });
  const sum = out.slice(out.indexOf('<summary'), out.indexOf('</summary>'));
  assert.match(sum, /4 files/);
  assert.match(sum, /aria-label="249 lines added, 15 removed"/);
  const rows = out.slice(out.indexOf('<ul role="list"'), out.indexOf('</ul>'));
  assert.equal([...rows.matchAll(/data-diffstat=/g)].length, 4, 'a diffstat per file');
  assert.doesNotMatch(rows, /aria-label="\d+ lines added"/, 'the old bare +n −n spans are gone');
});

test('IR-2 tests line: LED + words, "see CI" on a failure, no icon', () => {
  const ci = 'https://github.com/alexsergivan/kwant/actions/runs/2';
  const t = (state, links = {}) => {
    const out = html(m.LandedCard, { card: landed(fx.LANDED_288, { tests: { state, summary: '3 checks passed' }, links: { ...fx.LANDED_288.links, ...links } }) });
    return out.slice(out.indexOf('data-tests='), out.indexOf('data-tests=') + 600);
  };
  assert.match(t('passed'), /data-tests="passed" class="[^"]*text-ok[^"]*"><span aria-hidden="true" data-s="ok" class="dv-led"><\/span>/);
  assert.match(t('passed'), />Tests passed</);
  const failed = t('failed', { ci });
  assert.match(failed, /data-s="bad"/);
  assert.match(failed, /Tests failed · <a href="https:\/\/github\.com\/alexsergivan\/kwant\/actions\/runs\/2"[^>]*>see CI<\/a>/);
  assert.match(t('failed', { ci: null }), /<span>Tests failed<\/span>/);
  assert.match(t('none'), /data-s="off"[^]*No tests ran/);
  assert.match(t('unknown'), /data-s="off"[^]*Tests not known/);
  assert.doesNotMatch(t('passed'), /lucide-check|lucide-x|lucide-minus/);
});

test('IR-2 link words follow the forge, not GitHub by default (one source: the forge prop, QA 0.7.1 F2)', () => {
  assert.equal(m.forgeName, undefined, 'no second, host-derived source for the link word');
  const card = landed(fx.LANDED_289, { links: { pr: 'https://gitlab.com/o/r/-/merge_requests/289', commit: null, ci: null } });
  const gl = html(m.LandedCard, { card, forge: 'gitlab' });
  assert.match(gl, /Open on GitLab/);
  assert.doesNotMatch(gl, /GitHub|Open pull request/);
  assert.match(html(m.LandedCard, { card, forge: 'gitea' }), /Open on Gitea/);
  assert.match(html(m.LandedCard, { card }), /Open pull request/, 'forge unknown: plain words, never a guessed GitHub');
  const list = html(m.LandedList, { cards: [card], reviews: [], now: fx.FIXTURE_NOW, forge: 'gitlab' });
  assert.match(list, /Open on GitLab/, 'the list passes forge down');
  assert.match(list, /dv-rail/);
});

test('IR-4 rail node: passed teal, failed rose with the × cut-out, none/unknown hollow, pending pings', () => {
  assert.equal(m.landedNode({ status: 'ready', tests: { state: 'passed' } }), 'ok');
  assert.equal(m.landedNode({ status: 'ready', tests: { state: 'failed' } }), 'bad');
  assert.equal(m.landedNode({ status: 'ready', tests: { state: 'none' } }), 'off');
  assert.equal(m.landedNode({ status: 'ready', tests: { state: 'unknown' } }), 'off');
  assert.equal(m.landedNode({ status: 'pending', tests: { state: 'passed' } }), 'run');
  const css = readFileSync(new URL('../src/styles/modern.css', import.meta.url), 'utf8');
  assert.match(css, /\.dv-node\[data-s="bad"\]::after \{[^}]*clip-path: polygon/, 'the × cut-out is a shape, not only colour');
  assert.match(css, /\.dv-node\[data-s="run"\]::before \{[^}]*animation: dv-ping/);
});

test('IR-4 rail: one node per card, a merge arc only for a PR, day labels on the rail, nothing for a lone card', () => {
  const cards = [landed(fx.LANDED_303), landed(fx.LANDED_288), landed(fx.LANDED_DIRECT), landed(fx.LANDED_NO_TEXT)];
  const list = html(m.LandedList, { cards, reviews: [], now: NOW });
  assert.match(list, /^<div data-dev-landed-list="true" class="dv-rail /);
  const nodes = [...list.matchAll(/data-rail-node="(\w+)"/g)].map((x) => x[1]);
  assert.equal(nodes.length, cards.length);
  assert.equal([...list.matchAll(/data-rail-arc/g)].length, cards.filter((c) => c.pr !== null).length, 'direct push: node only');
  const direct = list.slice(list.indexOf('data-commit="c0ffee1"'));
  assert.doesNotMatch(direct.slice(0, direct.indexOf('<div')), /data-rail-arc/);
  assert.match(list, /aria-hidden="true" data-rail-node="\w+" data-s="\w+" class="dv-node"/, 'the node is decoration; the tests line says it in words');
  const days = [...list.matchAll(/data-rail-day="true"[^>]*>([^<]+)</g)].map((x) => x[1]);
  assert.ok(days.includes('Today') && days.includes('Yesterday'), days.join());
  const lone = html(m.LandedCard, { card: landed(fx.LANDED_288) });
  assert.doesNotMatch(lone, /dv-node|dv-arc/, 'off the rail, no node');
});
