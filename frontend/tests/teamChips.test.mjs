// 18e team-chips-plain: a team chat names the team once, long names stay inside their pill, and Approve hands
// focus back to the composer on desktop.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { loadBundled } from './bundle.mjs';

const m = await loadBundled(`
  export { answerTarget } from './src/botHome';
  export { refocusAfterApprove } from './src/approvalAsk';
`);
const chat = readFileSync(new URL('../src/components/ChatView.tsx', import.meta.url), 'utf8');

test('the team is named once, without "(Team)"; the Ask row\'s team chip went with composer-diet a', () => {
  assert.doesNotMatch(chat, /\(Team\)/);
  assert.doesNotMatch(chat, /Ask whole team|data-team-chip/);
});

test('a long team name truncates inside the Answers chip; the full name stays in the title', () => {
  const pill = chat.slice(chat.indexOf('data-answer-target'), chat.indexOf('data-answer-name') + 200);
  // Beside the hint text it takes at most 26rem or 62% of the row (it now also names the lead); alone, the whole row.
  assert.match(chat, /kw-chip min-w-0 \$\{showHintText \? 'max-w-\[min\(26rem,62%\)\]' : 'max-w-full'\}/);
  assert.match(pill, /shrink-0 whitespace-nowrap text-zinc-400 font-normal">Answers:/);
  assert.match(pill, /className="min-w-0 truncate" data-answer-name/);
  assert.match(chat, /title=\{target\.detail\}\s*data-answer-target/);
  const t = m.answerTarget({ kind: 'team', text: '', teamName: 'Booktun.ing Marketing Operations Team', names: [] });
  assert.equal(t.label, '@Booktun.ing Marketing Operations Team');
  assert.match(t.detail, /^@Booktun\.ing Marketing Operations Team answers\./);
});

test('focus returns to the composer after Approve on desktop only when it was lost or still in the row', () => {
  // The row unmounted with the reply: focus fell to <body>.
  assert.equal(m.refocusAfterApprove({ phone: false, focusLost: true, inRow: false }), true);
  // A failed send keeps the row; focus is still on Approve.
  assert.equal(m.refocusAfterApprove({ phone: false, focusLost: false, inRow: true }), true);
  // The person already moved on (Tab to another control, clicked the sidebar): leave it.
  assert.equal(m.refocusAfterApprove({ phone: false, focusLost: false, inRow: false }), false);
  // Never on a phone: focusing the field would open the keyboard.
  assert.equal(m.refocusAfterApprove({ phone: true, focusLost: true, inRow: false }), false);
  assert.equal(m.refocusAfterApprove({ phone: true, focusLost: false, inRow: true }), false);
  const approve = chat.slice(chat.indexOf('const approve = async'), chat.indexOf('const react = async'));
  assert.match(approve, /finally \{[\s\S]*refocusAfterApprove\(\{ phone, focusLost, inRow \}\)\) focusComposer\(\)/);
  assert.match(approve, /closest\?\.\('\[data-approve-row\]'\)/);
});
