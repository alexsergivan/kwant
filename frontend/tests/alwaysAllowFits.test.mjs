import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

// 29r: on a 320 px phone "Always allow" (and the longer "Always this session") broke onto two or three lines,
// so the Deny / Always row read as more than two buttons. The labels now never wrap; the row itself wraps
// instead (Deny and Always share it in equal halves while both labels fit, and the second drops to its own
// full-width line when they don't). Measured widths: autonomy/always-allow-fits-320/impl.md.

globalThis.localStorage ??= { getItem: () => null, setItem: () => {} };

const m = await loadBundled(`
  export { ModernApprovalCard } from './src/components/ModernApprovalCard';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export { createElement } from 'react';
`);

const mcp = {
  id: 'ap1',
  tool_name: 'mcp__github__create_pull_request',
  details: { source: 'mcp', server: 'github', tool: 'create_pull_request', kwant_name: 'TechLead', arguments: { repo: 'acme/site' } },
};
const builtin = {
  id: 'ap2',
  tool_name: 'Write',
  details: { source: 'claude_builtin', tool: 'Write', kwant_name: 'TechLead', arguments: { file_path: '/home/me/notes.md', content: 'hi' } },
};

const row = (approval) => {
  const html = m.renderToStaticMarkup(m.createElement(m.ModernApprovalCard, { approval, onResolve: () => {} }));
  const doc = new JSDOM(html).window.document;
  return doc.querySelector('[data-approval-deny-row]');
};
const cls = (el) => (el.getAttribute('class') ?? '').split(/\s+/);

for (const [what, approval, label] of [['MCP', mcp, 'Always allow'], ['built-in', builtin, 'Always this session']]) {
  test(`29r (${what}): "${label}" and Deny never break their label; the row wraps instead`, () => {
    const r = row(approval);
    assert.ok(r, 'the Deny / Always row is marked');
    assert.ok(cls(r).includes('flex') && cls(r).includes('flex-wrap'), `row wraps: ${r.getAttribute('class')}`);
    assert.ok(!cls(r).some((c) => c.startsWith('grid')), 'no fixed two-column grid that squeezes a label');
    const buttons = [...r.querySelectorAll('button')];
    assert.deepEqual(buttons.map((b) => b.textContent), ['Deny', label], 'same buttons, same order, same words');
    for (const b of buttons) {
      assert.ok(cls(b.querySelector('span')).includes('whitespace-nowrap'), `${b.textContent}: label stays on one line`);
      for (const c of ['flex-1', 'basis-0', 'sm:flex-auto', 'min-h-[44px]']) assert.ok(cls(b).includes(c), `${b.textContent}: has ${c}`);
    }
  });
}
