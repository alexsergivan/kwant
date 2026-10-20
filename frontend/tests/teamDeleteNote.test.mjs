// Team delete on a phone (2026-10-20): the modal is scrolled down to the Delete button, so the
// "Nothing was changed" note after "Not now" sits right above that button, not at the top.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

// The sound effects module reads its mute flag on load.
globalThis.localStorage ??= { getItem: () => null, setItem: () => {} };

const m = await loadBundled(`
  export { createElement } from 'react';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export { ManageTeamModal, DeleteNote } from './src/components/ManageTeamModal';
  export { PERSON_CANCELLED_TEXT } from './src/auth';
`);

test('the delete note is an alert with the reason, and renders nothing without one', () => {
  const html = m.renderToStaticMarkup(m.createElement(m.DeleteNote, { text: m.PERSON_CANCELLED_TEXT }));
  assert.match(html, /role="alert"/);
  assert.ok(html.includes('Nothing was changed'));
  assert.equal(m.renderToStaticMarkup(m.createElement(m.DeleteNote, { text: null })), '');
});

test('the team modal has no delete note before a delete is tried', () => {
  const team = { id: 't1', name: 'crew', lead_kwant_id: 'k1', member_kwants: ['k1'] };
  const html = m.renderToStaticMarkup(
    m.createElement(m.ManageTeamModal, {
      isOpen: true,
      team,
      kwants: [{ id: 'k1', name: 'Ada' }],
      onClose: () => {},
      onUpdateTeam: async () => {},
      onDeleteTeam: async () => {},
    }),
  );
  assert.ok(html.includes('Delete Team'));
  assert.ok(!html.includes('team-delete-note'));
});
