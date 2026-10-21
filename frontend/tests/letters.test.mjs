// Camera-to-Completed (idea 05): a letter becomes a Loose ends case. Uncertain fields are never shown
// as checked, a deadline is only ever YYYY-MM-DD, reminders wait for a confirmed and certain deadline,
// a busy server reads calmly, an unreadable letter says so, and a paper reply is copy only.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const m = await loadBundled(`export * from './src/letters'; export { LooseEndsError } from './src/looseEnds';`);
const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { LetterDetail, FieldRow, ListField, LetterReminders, LetterDraft, AddLetterForm } from './src/components/LetterView';
  import { LooseCaseCard } from './src/components/LooseEndsView';
  const r = (C) => (props) => renderToStaticMarkup(React.createElement(C, props));
  export const detail = r(LetterDetail);
  export const row = r(FieldRow);
  export const list = r(ListField);
  export const reminders = r(LetterReminders);
  export const draft = r(LetterDraft);
  export const add = r(AddLetterForm);
  export const card = (props) => renderToStaticMarkup(React.createElement('ul', null, React.createElement(LooseCaseCard, props)));
`);

const noop = () => {};
const save = async () => null;
const f = (value, over = {}) => ({ value, quote: `“${value}”`, uncertain: false, confirmed: false, ...over });

// Shaped like DocumentView in src/server/document_routes.rs.
const letterCase = (over = {}) => ({
  id: 'd1', status: 'open', kind: 'document_request', conv_kind: 'document', source: 'document', source_id: 'd1',
  title: 'Letter from Thames Water', counterpart: null, counterpart_name: 'Thames Water', reference: 'TW-88213',
  link: null, why: 'Pay the outstanding balance', evidence: [],
  draft: { channel: 'letter', to: 'Thames Water', subject: 'Re: TW-88213', body: 'Dear Thames Water,\nre TW-88213.', mailto: null, open_url: null, kwant_thread_id: null, generated_by: 'template', created_at: '2026-10-20T08:00:00Z' },
  age_days: 0, last_activity_at: '2026-10-20T08:00:00Z', snoozed_until: null, waiting_since: null, follow_ups: 0,
  deadline: null, resolution: null, dismiss_reason: null,
  ...over,
});
const extraction = (over = {}) => ({
  status: 'read', method: 'pdf_text+model_vision', note: null,
  fields: {
    sender: f('Thames Water', { confirmed: true }),
    request: f('Pay the outstanding balance'),
    reply_to: null,
    deadline: f('2026-11-14', { uncertain: true }),
    references: [f('TW-88213')],
    amounts: [f('GBP 142.60')],
    checklist: [f('Pay the bill', { done: true, confirmed: true }), f('Send meter reading', { done: false })],
  },
  text: 'Water bill', read_by: 'cli',
  pages: [
    { id: 'a1', name: 'water-bill.pdf', mime: 'application/pdf', url: '/api/attachments/a1/content' },
    { id: 'a2', name: 'letter-page2.png', mime: 'image/png', url: '/api/attachments/a2/content' },
  ],
  updated_at: '2026-10-20T08:00:00Z',
  ...over,
});
const view = (over = {}, ext = {}) => ({ case: letterCase(over), extraction: extraction(ext), reminders: [] });
const detailProps = (v, over = {}) => ({ view: v, loading: false, error: null, onSave: save, onDone: noop, onReopen: noop, onDelete: noop, onBack: noop, onRetry: noop, ...over });

test('a field is Confirmed only when confirmed and not uncertain; uncertain is never hidden', () => {
  assert.equal(m.fieldState(null), 'empty');
  assert.equal(m.fieldState(f('')), 'empty');
  assert.equal(m.fieldState(f('x')), 'unconfirmed');
  assert.equal(m.fieldState(f('x', { uncertain: true })), 'uncertain');
  assert.equal(m.fieldState(f('x', { confirmed: true })), 'confirmed');
  assert.equal(m.fieldState(f('x', { confirmed: true, uncertain: true })), 'uncertain');

  const uncertain = ui.row({ fkey: 'deadline', label: 'Deadline', hint: 'YYYY-MM-DD', field: f('2026-11-14', { uncertain: true }), disabled: false, onSave: save });
  assert.match(uncertain, /data-state="uncertain"/);
  assert.match(uncertain, />Uncertain</);
  assert.match(uncertain, />.*Confirm</);
  assert.match(uncertain, /“2026-11-14”/, 'the quote is shown next to the value');

  const ok = ui.row({ fkey: 'sender', label: 'From', hint: '', field: f('Thames Water', { confirmed: true }), disabled: false, onSave: save });
  assert.match(ok, />Confirmed</);
  assert.doesNotMatch(ok, />Uncertain</);
  assert.doesNotMatch(ok, /> Confirm</, 'a confirmed field offers no Confirm');
  assert.match(ok, /Edit/);
  assert.match(ok, /Clear/);

  const none = ui.row({ fkey: 'reply_to', label: 'Reply to', hint: '', field: null, disabled: false, onSave: save });
  assert.match(none, /Not found in the letter/);
  assert.match(none, /Add/);
  assert.doesNotMatch(none, /Clear/);

  const noQuote = ui.row({ fkey: 'sender', label: 'From', hint: '', field: { value: 'ACME', quote: '', uncertain: true, confirmed: false }, disabled: false, onSave: save });
  assert.match(noQuote, /No matching words were found in the letter/);
});

test('the PATCH bodies match the API: {} confirms, {value} sets, null clears, lists go whole', () => {
  assert.deepEqual(m.confirmBody('deadline'), { deadline: {} });
  assert.deepEqual(m.setBody('deadline', ' 2026-11-14 '), { deadline: { value: '2026-11-14' } });
  assert.deepEqual(m.clearBody('sender'), { sender: null });
  assert.deepEqual(m.listBody('checklist', [{ value: 'Pay', done: true }, { value: 'Call' }]), { checklist: [{ value: 'Pay', done: true }, { value: 'Call', done: false }] });
  assert.deepEqual(m.listBody('amounts', [{ value: 'GBP 142.60', done: true }]), { amounts: [{ value: 'GBP 142.60' }] }, 'done only on checklist');
  assert.deepEqual(m.listBody('checklist', [{ value: 'Pay', done: true }], true), { checklist: [{ value: 'Pay', done: true, confirmed: true }] }, 'confirm all / save');
  assert.deepEqual(m.listBody('references', [{ value: 'TW-1' }], true), { references: [{ value: 'TW-1', confirmed: true }] });
});

test('a deadline must be a real YYYY-MM-DD date', () => {
  for (const ok of ['2026-11-14', '2028-02-29', ' 2026-01-01 ']) assert.equal(m.isIsoDate(ok), true, ok);
  for (const bad of ['14/11/2026', '2026-11-31', '2027-02-29', '2026-1-4', '20261114', '2026-13-01', 'next Friday', '']) assert.equal(m.isIsoDate(bad), false, bad);
  assert.match(m.fieldValueError('deadline', '14 Nov 2026'), /YYYY-MM-DD/);
  assert.equal(m.fieldValueError('deadline', '2026-11-14'), null);
  assert.match(m.fieldValueError('amounts', '142.60'), /currency/);
  assert.equal(m.fieldValueError('amounts', 'GBP 142.60'), null);
  assert.equal(m.fieldValueError('amounts', '£142.60'), null);
  assert.match(m.fieldValueError('sender', '   '), /Clear/);
  assert.match(m.fieldValueError('sender', 'x'.repeat(501)), /500/);

  // A model's deadline that isn't ISO can't be confirmed as it is: it must be edited first.
  const html = ui.row({ fkey: 'deadline', label: 'Deadline', hint: 'YYYY-MM-DD', field: f('14 November', { uncertain: true }), disabled: false, onSave: save });
  assert.match(html, /isn’t a YYYY-MM-DD date yet/);
  assert.match(html, /<button[^>]*disabled=""[^>]*>[^<]*<svg[^]*?Confirm/);
  // The edit box says the format.
  const editing = ui.row({ fkey: 'deadline', label: 'Deadline', hint: 'YYYY-MM-DD', field: f('2026-11-14'), disabled: false, onSave: save, initialEditing: true });
  assert.match(editing, /placeholder="YYYY-MM-DD"/);
  assert.match(editing, /Year-month-day, for example 2026-11-14/);
});

test('reminders wait for a confirmed, certain deadline, then list 7/1/0 days with due or sent', () => {
  assert.equal(m.reminderGate(null).on, false);
  assert.match(m.reminderGate(null).text, /No deadline yet/);
  assert.equal(m.reminderGate(f('2026-11-14')).on, false);
  assert.match(m.reminderGate(f('2026-11-14')).text, /once you confirm the deadline/);
  assert.equal(m.reminderGate(f('2026-11-14', { uncertain: true })).on, false);
  assert.match(m.reminderGate(f('2026-11-14', { uncertain: true })).text, /uncertain, so there are no reminders/);
  assert.equal(m.reminderGate(f('2026-11-14', { confirmed: true })).on, true);

  const off = ui.reminders({ deadline: f('2026-11-14', { uncertain: true }), reminders: [], closed: false });
  assert.match(off, /data-reminders="off"/);
  assert.match(off, /uncertain/);

  const on = ui.reminders({
    deadline: f('2026-11-14', { confirmed: true }),
    reminders: [
      { offset_days: 0, due_at: '2026-11-14T09:00:00Z', fired_at: null },
      { offset_days: 7, due_at: '2026-11-07T09:00:00Z', fired_at: '2026-11-07T09:02:00Z' },
      { offset_days: 1, due_at: '2026-11-13T09:00:00Z', fired_at: null },
    ],
    closed: false,
  });
  assert.match(on, /data-reminders="on"/);
  const order = ['7 days before', '1 day before', 'On the day'].map((t) => on.indexOf(t));
  assert.ok(order.every((i) => i > 0) && order[0] < order[1] && order[1] < order[2], 'listed 7, 1, 0');
  assert.match(on, />Sent /);
  assert.equal((on.match(/>Due</g) || []).length, 2);
  assert.match(on, /counts only/);

  assert.match(ui.reminders({ deadline: f('2026-11-14', { confirmed: true }), reminders: [], closed: true }), /closed, so its reminders are off/);
});

test('lists: amounts with currency, checklist ticks, and saving says it confirms the whole list', () => {
  const amounts = ui.list({ fkey: 'amounts', label: 'Amounts', hint: 'GBP 142.60', empty: 'No amounts found.', items: [f('GBP 142.60', { uncertain: true })], disabled: false, onSave: save });
  assert.match(amounts, /GBP 142.60/);
  assert.match(amounts, />Uncertain</);
  assert.match(amounts, /Confirm all/);

  const check = ui.list({ fkey: 'checklist', label: 'Checklist', hint: '', empty: '', items: [f('Pay the bill', { done: true, confirmed: true }), f('Send meter reading')], disabled: false, onSave: save });
  assert.equal((check.match(/type="checkbox"/g) || []).length, 2);
  assert.match(check, /checked=""[^>]*>[^]*?line-through[^>]*>Pay the bill/);
  assert.doesNotMatch(check, /Ticking an item also confirms/);

  const editing = ui.list({ fkey: 'references', label: 'References', hint: '', empty: '', items: [f('TW-88213')], disabled: false, onSave: save, initialEditing: true });
  assert.match(editing, /Saving confirms every entry in this list/);
  assert.match(ui.list({ fkey: 'references', label: 'References', hint: '', empty: 'No reference numbers found.', items: [], disabled: false, onSave: save }), /No reference numbers found/);
});

test('reading shows progress and no fields; every status is said honestly', () => {
  const running = ui.detail(detailProps(view({ title: 'Letter (reading…)' }, { status: 'running', method: null, fields: {} })));
  assert.match(running, /data-read-status="running"/);
  assert.match(running, /Reading the letter…/);
  assert.match(running, /role="progressbar"/);
  assert.match(running, /fields appear here once the letter has been read/);
  assert.doesNotMatch(running, /data-field=/);
  assert.equal(m.stillReading(view({}, { status: 'running' })), true);
  assert.equal(m.stillReading(view({}, { status: 'read' })), false);

  assert.match(m.readingText(extraction({ method: 'model_vision' })).body, /by your Claude model/);
  const textOnly = m.readingText(extraction({ status: 'text_only', note: 'no model can read it' }));
  assert.match(textOnly.title, /Only the PDF text/);
  assert.match(textOnly.body, /No model can read it\./);

  const unreadable = ui.detail(detailProps(view({}, { status: 'unreadable', method: null, note: 'kwant restarted while reading it', fields: {} })));
  assert.match(unreadable, /data-read-status="unreadable"/);
  assert.match(unreadable, /This letter couldn’t be read/);
  assert.match(unreadable, /kwant restarted while reading it\./);
  assert.match(unreadable, /Fill in the fields yourself from the pages/);
  // Still fillable by hand, and the pages are still shown.
  assert.match(unreadable, /data-field="sender" data-state="empty"/);
  assert.match(unreadable, /Page 2 of the letter/);
  // Nothing was read, so no field claims it was "not found" in the letter.
  assert.doesNotMatch(unreadable, /Not found in the letter|No reference numbers found/);
  assert.match(unreadable, /Not filled in yet/);
  assert.match(unreadable, /None added yet/);

  // The server's note already asks for the fields: it isn't said twice.
  const failed = m.readingText(extraction({ status: 'unreadable', note: 'the model could not read it (the claude command exited with 1): fill in the fields yourself' }));
  assert.equal(failed.body.match(/fill in the fields/gi).length, 1);
});

test('the pages sit next to the fields: photos inline, PDFs as a link, plus what kwant read', () => {
  const html = ui.detail(detailProps(view()));
  assert.match(html, /<img src="\/api\/attachments\/a2\/content" alt="Page 2 of the letter"/);
  assert.match(html, /href="\/api\/attachments\/a1\/content"[^>]*>[^]*?Open PDF/);
  assert.match(html, /What was read from it/);
  assert.match(html, /lg:grid-cols-\[minmax\(0,1fr\)_minmax\(0,1fr\)\]/, 'side by side on a wide screen');
  assert.ok(html.indexOf('The letter') < html.indexOf('What it says'), 'pages first, so a phone shows the photo above the fields');
  assert.match(html, /1 field needs a look/, 'only the uncertain deadline needs the person');
});

test('sure fields arrive confirmed with an auto mark, still editable; uncertain ones still need a tap', () => {
  const auto = ui.row({ fkey: 'sender', label: 'From', hint: '', field: f('Thames Water', { confirmed: true, auto: true }), disabled: false, onSave: save });
  assert.match(auto, /data-state="confirmed"/);
  assert.match(auto, /data-auto="true"/);
  assert.match(auto, />auto</);
  assert.match(auto, /confirmed automatically, not by you/, 'screen readers hear who confirmed it');
  assert.doesNotMatch(auto, /> Confirm</, 'nothing to tap');
  assert.match(auto, /Edit/);
  assert.match(auto, /Clear/);
  assert.match(auto, /“Thames Water”/, 'the quote stays visible so it can be checked');

  const mine = ui.row({ fkey: 'sender', label: 'From', hint: '', field: f('Thames Water', { confirmed: true }), disabled: false, onSave: save });
  assert.doesNotMatch(mine, /data-auto/, 'a field the person confirmed has no auto mark');

  const unsure = ui.row({ fkey: 'deadline', label: 'Deadline', hint: 'YYYY-MM-DD', field: f('2026-11-14', { uncertain: true }), disabled: false, onSave: save });
  assert.doesNotMatch(unsure, /data-auto/);
  assert.match(unsure, /> Confirm</);

  // A sure deadline starts reminders on its own; an uncertain one never does
  assert.equal(m.reminderGate(f('2026-11-14', { confirmed: true, auto: true })).on, true);
  assert.equal(m.reminderGate(f('2026-11-14', { uncertain: true })).on, false);
  assert.equal(m.isAuto(f('x', { confirmed: true, auto: true, uncertain: true })), false, 'uncertain beats auto');

  const list = ui.list({ fkey: 'amounts', label: 'Amounts', hint: '', empty: '', items: [f('GBP 142.60', { confirmed: true, auto: true }), f('GBP 9.99', { uncertain: true })], disabled: false, onSave: save });
  assert.equal((list.match(/data-auto="true"/g) ?? []).length, 1);
  assert.match(list, /Not all checked/);

  const sure = { sender: f('A', { confirmed: true, auto: true }), deadline: f('2026-11-14', { confirmed: true, auto: true }), amounts: [f('GBP 1.00', { uncertain: true })] };
  assert.equal(m.needsLook(sure), 1);
  const html = ui.detail(detailProps(view({}, { fields: { ...sure, amounts: [] } })));
  assert.match(html, /Nothing needs a look/);
});

test('the draft: a paper letter is copy only; an email has a mailto; nothing is sent either way', () => {
  const paper = ui.draft({ c: letterCase() });
  assert.match(paper, /data-draft-channel="letter"/);
  assert.match(paper, /Reply by post to Thames Water/);
  assert.match(paper, /Nothing is sent from kwant bots/);
  assert.match(paper, /print or write it out/);
  assert.match(paper, /Copy reply/);
  assert.doesNotMatch(paper, /mailto:|Open in my mail app/);

  const email = ui.draft({
    c: letterCase({ draft: { ...letterCase().draft, channel: 'email', to: 'billing@water.example', mailto: 'mailto:billing@water.example?subject=Re' } }),
  });
  assert.match(email, /Reply by email to billing@water.example/);
  assert.match(email, /href="mailto:billing@water.example\?subject=Re"/);
  assert.match(email, /Open in my mail app/);
  assert.match(email, /Nothing is sent from kwant bots/);

  // A letter channel never gets a link, even if the server sent one.
  const odd = ui.draft({ c: letterCase({ draft: { ...letterCase().draft, mailto: 'mailto:x@y.example' } }) });
  assert.doesNotMatch(odd, /mailto:/);
});

test('a busy server reads calmly; bad files and counts say what to change', () => {
  const busy = m.letterErrorText(new m.LooseEndsError('Another letter is being read: try again in a minute.', 409, false, false, 'busy'));
  assert.equal(busy.calm, true);
  assert.match(busy.text, /Another letter is being read right now/);
  assert.doesNotMatch(busy.text, /409|HTTP|error/i);
  const used = m.letterErrorText(new m.LooseEndsError('A page was already used: upload it again.', 409));
  assert.equal(used.calm, false);
  assert.equal(m.letterErrorText(new m.LooseEndsError('Pages must be photos (PNG, JPEG, WebP) or PDFs.', 400)).calm, false);

  const page = (name, type = 'image/png', size = 1000) => ({ name, type, size });
  assert.equal(m.checkLetterFiles([page('a.png'), page('b.pdf', 'application/pdf'), page('c.jpg', 'image/jpeg'), page('d.webp', 'image/webp')]), null);
  assert.equal(m.checkLetterFiles([page('scan.PDF', '')]), null, 'no MIME: the extension decides');
  assert.match(m.checkLetterFiles([]), /Add a photo or a PDF/);
  assert.match(m.checkLetterFiles(Array.from({ length: 6 }, (_, i) => page(`${i}.png`))), /at most 5 pages\. You picked 6: remove 1/);
  assert.match(m.checkLetterFiles([page('notes.txt', 'text/plain')]), /notes\.txt isn’t a photo or a PDF/);
  assert.match(m.checkLetterFiles([page('anim.gif', 'image/gif')]), /PNG, JPEG, WebP or PDF/);
  assert.match(m.checkLetterFiles([page('huge.jpg', 'image/jpeg', 25 * 1024 * 1024)]), /too big/);
  assert.match(m.checkLetterFiles([page('empty.png', 'image/png', 0)]), /empty/);

  const form = ui.add({ onCreated: noop, onCancel: noop, initialCamera: true });
  assert.match(form, /Take a photo/);
  assert.match(form, /capture="environment"/);
  assert.match(form, /accept="image\/png,image\/jpeg,image\/webp,application\/pdf/);
  assert.match(form, /Nothing is sent to anyone/);
  assert.match(form, /<button type="submit" disabled=""/, 'nothing to read yet');
  assert.doesNotMatch(ui.add({ onCreated: noop, onCancel: noop, initialCamera: false }), /Take a photo/);
});

test('delete asks first; a 409 while reading is calm and nothing is half-deleted', () => {
  const html = ui.detail(detailProps(view(), { initialConfirmDelete: true }));
  assert.match(html, /data-confirm-delete/);
  assert.match(html, /can’t be undone/);
  assert.match(html, /Delete it/);
  assert.match(html, /Keep it/);
  const reading = m.deleteErrorText(new m.LooseEndsError('The letter is still being read.', 409));
  assert.equal(reading.calm, true);
  assert.match(reading.text, /still being read/);
  assert.match(m.deleteErrorText(new Error('Network down.')).text, /Nothing was deleted/);
});

test('done and reopen: an open letter offers Mark done, a closed one Reopen with reminders off', () => {
  const open = ui.detail(detailProps(view()));
  assert.match(open, /Mark done/);
  assert.doesNotMatch(open, /Reopen/);
  const done = ui.detail(detailProps({ ...view({ status: 'closed' }), reminders: [] }));
  assert.match(done, /Reopen/);
  assert.doesNotMatch(done, /Mark done/);
  assert.match(done, /closed, so its reminders are off/);
  assert.doesNotMatch(done, /data-draft-channel/, 'no reply draft on a closed letter');
});

test('a letter card in the Loose ends list says Letter and opens the letter', () => {
  const html = ui.card({ c: letterCase(), busy: false, onAction: noop, onOpenLetter: noop });
  assert.match(html, />Letter</);
  assert.match(html, /Letter to deal with/);
  assert.match(html, /data-open-letter/);
  assert.doesNotMatch(ui.card({ c: { ...letterCase(), source: 'imap', kind: 'awaiting_reply' }, busy: false, onAction: noop, onOpenLetter: noop }), /data-open-letter/);
});
