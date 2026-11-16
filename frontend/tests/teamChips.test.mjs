// 18e team-chips-plain: a team chat names the team once, long names stay inside their pill, and Approve hands
// focus back to the composer on desktop.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { loadBundled } from './bundle.mjs';

const m = await loadBundled(`
  export { teamChipLabel, teamChipTitle, answerTarget } from './src/botHome';
  export { refocusAfterApprove } from './src/approvalAsk';
`);
const chat = readFileSync(new URL('../src/components/ChatView.tsx', import.meta.url), 'utf8');

test('the team chip names the team once, without "(Team)"', () => {
  assert.equal(m.teamChipLabel('Booktun.ing Team'), '@Booktun.ing Team');
  assert.equal(m.teamChipLabel('helpers'), '@helpers');
  assert.equal(m.teamChipLabel('  Booktun.ing Marketing Operations Team '), '@Booktun.ing Marketing Operations Team');
  assert.doesNotMatch(chat, /\(Team\)/);
  assert.doesNotMatch(chat, /Ask whole team/);
});

test('the team chip title says "the whole … team" without doubling "team"', () => {
  assert.equal(m.teamChipTitle('Booktun.ing Team'), 'Ask the whole Booktun.ing Team');
  assert.equal(m.teamChipTitle('helpers'), 'Ask the whole helpers team');
  assert.equal(m.teamChipTitle('Ops TEAM'), 'Ask the whole Ops TEAM');
  assert.doesNotMatch(m.teamChipTitle('Booktun.ing Team'), /team team/i);
});

test('a long team name truncates inside the chip and the Answers pill; the full name stays in the title', () => {
  const chip = chat.slice(chat.indexOf('data-team-chip'), chat.indexOf('data-team-chip') + 400);
  assert.match(chat, /bg-sky-950\/60[^']*'/);
  assert.equal((chat.match(/max-w-\[14rem\] whitespace-nowrap flex items-center gap-1\.5 px-2\.5[^']*bg-sky-950\/60/g) ?? []).length, 2);
  assert.match(chip, /className="min-w-0 truncate">\{teamChipLabel\(teamName\)\}/);
  assert.match(chat, /title=\{teamChipTitle\(teamName\)\}/);
  const pill = chat.slice(chat.indexOf('data-answer-target'), chat.indexOf('data-answer-name') + 200);
  // Beside the hint text it takes at most 22rem or 60% of the row; alone (phone) it may use the whole row.
  assert.match(chat, /kw-chip min-w-0 \$\{showHintText \? 'max-w-\[min\(22rem,60%\)\]' : 'max-w-full'\}/);
  assert.match(pill, /shrink-0 whitespace-nowrap text-zinc-400 font-normal">Answers:/);
  assert.match(pill, /className="min-w-0 truncate" data-answer-name/);
  // answerTarget's words are unchanged (18e doesn't touch them): the title keeps the full name.
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
