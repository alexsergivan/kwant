// done-check samples for tests and shots. The partly, checked and not-checked objects are Vega's real
// GET /api/inbox output from #566 (autonomy/done-check/impl.md, "Real sample"); the failed one and the long
// URL are made-up data in the same shape.
import type { Checked } from './doneCheck';

export const CHECKED_PARTLY: Checked = {
  state: 'partly',
  items: [
    { what: 'https://kwant.si/pricing', how: 'opened the address', result: 'passed', evidence: 'HTTP 200 in 212 ms' },
    { what: 'npm run check-prices', how: 'ran the command', result: 'not_checked', evidence: null },
    {
      what: 'The three plan prices match the plan sheet',
      how: 'task criterion, met with evidence',
      result: 'passed',
      evidence: 'Pricing page preview: https://kwant.si/pricing',
    },
    { what: 'The FAQ mentions the yearly discount', how: 'task criterion, marked met with no evidence (a claim)', result: 'not_checked', evidence: null },
  ],
  items_dropped: 0,
  line: 'Partly checked: 1 of 2 checks passed, 1 of 2 criteria have evidence.',
};

export const CHECKED_OK: Checked = {
  state: 'checked',
  items: [{ what: 'The link you asked for', how: 'kwant compared the reply with what you asked for', result: 'passed', evidence: 'the link is in the reply' }],
  items_dropped: 0,
  line: 'Checked: the link you asked for is in the reply.',
};

export const CHECKED_NONE: Checked = { state: 'not_checked', items: [], items_dropped: 0, line: 'Not checked: nothing verified this result.' };

export const CHECKED_FAILED: Checked = {
  state: 'failed',
  items: [
    {
      what: 'cargo test --locked',
      how: 'ran the command',
      result: 'failed',
      evidence: 'test inbox_done::a_failed_check_fails_the_item ... FAILED\nthread panicked at src/server/inbox_done.rs:412:5',
    },
    { what: 'The product photos are 1200 px wide', how: 'task criterion, marked not met', result: 'failed', evidence: null },
    { what: 'https://shop.example/products/spring-blend', how: 'opened the address', result: 'passed', evidence: 'HTTP 200 in 340 ms' },
    { what: 'Review by Rigel', how: 'a teammate’s review', result: 'waiting', evidence: null },
  ],
  items_dropped: 2,
  line: 'Failed: 1 of 5 checks failed, the criterion is not met.',
};

/** A 120-character URL for the 390 px overflow check. */
export const LONG_URL = 'https://shop.example/products/spring-blend/variants/house-blend-1kg/photos/originals/2026-11-13/front-view-large.jpg?v=1';

// request-parts (autonomy/request-parts/ui-brief.md): the five cases for the parts rows. PARTS_NO_CHECK is the
// impl.md "JSON shape" sample, with the "…" in its line filled from impl.md's line examples; the rest are made-up
// data in the same shape (#619, vega-request-parts).
export const PARTS_NO_CHECK: Checked = {
  state: 'partly',
  items: [
    { what: 'test -f shared/sales/october-sales.pdf', how: 'ran the command', result: 'passed', evidence: 'exit 0' },
    { what: 'The PDF has the October totals', how: 'task criterion, met with evidence', result: 'passed', evidence: 'Page 1: October total 48,210 EUR' },
    { what: 'The PDF is in shared/sales/', how: 'task criterion, met with evidence', result: 'passed', evidence: 'shared/sales/october-sales.pdf, 212 KB' },
  ],
  items_dropped: 0,
  line: 'Partly checked: 2 of 3 parts checked; "send it to Ana": no check; test -f shared/sales/october-sales.pdf passed.',
  parts_checked: 2,
  parts_total: 3,
  parts_written: true,
  parts: [
    { text: 'Make the October sales PDF', result: 'passed', criteria: [{ id: 'c1', text: 'The PDF has the October totals', result: 'passed' }] },
    { text: 'put it in the shared folder', result: 'passed', criteria: [{ id: 'c2', text: 'The PDF is in shared/sales/', result: 'passed' }] },
    { text: 'send it to Ana', result: 'not_checked', criteria: [] },
  ],
};

export const PARTS_ALL_PASSED: Checked = {
  state: 'checked',
  items: [
    { what: 'The draft names all three new flavours', how: 'task criterion, met with evidence', result: 'passed', evidence: 'Spring Blend, House Decaf and Cold Brew Kit are in the second paragraph' },
    { what: 'The draft is under 150 words', how: 'task criterion, met with evidence', result: 'passed', evidence: '128 words' },
    { what: 'The draft is in the newsletter queue', how: 'task criterion, met with evidence', result: 'passed', evidence: 'Queued for Tuesday 9:00' },
  ],
  items_dropped: 0,
  line: 'Checked: 2 of 2 parts checked.',
  parts_checked: 2,
  parts_total: 2,
  parts_written: true,
  parts: [
    {
      text: 'Write a short newsletter about the three new flavours',
      result: 'passed',
      criteria: [
        { id: 'c1', text: 'The draft names all three new flavours', result: 'passed' },
        { id: 'c2', text: 'The draft is under 150 words', result: 'passed' },
      ],
    },
    { text: 'and queue it for Tuesday morning', result: 'passed', criteria: [{ id: 'c3', text: 'The draft is in the newsletter queue', result: 'passed' }] },
  ],
};

export const PARTS_ONE_FAILED: Checked = {
  state: 'failed',
  items: [
    { what: 'The PDF has the October totals', how: 'task criterion, marked not met', result: 'failed', evidence: 'The totals table stops at 30 September' },
    { what: 'The PDF is in shared/sales/', how: 'task criterion, met with evidence', result: 'passed', evidence: 'shared/sales/october-sales.pdf, 198 KB' },
    { what: 'Ana has the PDF', how: 'task criterion, met with evidence', result: 'passed', evidence: 'Sent to ana@example.com at 09:14' },
  ],
  items_dropped: 0,
  line: 'Failed: 2 of 3 parts checked; "Make the October sales PDF": not met.',
  parts_checked: 2,
  parts_total: 3,
  parts_written: true,
  parts: [
    { text: 'Make the October sales PDF', result: 'failed', criteria: [{ id: 'c1', text: 'The PDF has the October totals', result: 'failed' }] },
    { text: 'put it in the shared folder', result: 'passed', criteria: [{ id: 'c2', text: 'The PDF is in shared/sales/', result: 'passed' }] },
    { text: 'send it to Ana', result: 'passed', criteria: [{ id: 'c3', text: 'Ana has the PDF', result: 'passed' }] },
  ],
};

export const PARTS_ONE_WRITTEN: Checked = {
  state: 'checked',
  items: [{ what: 'The FAQ mentions the yearly discount', how: 'task criterion, met with evidence', result: 'passed', evidence: 'FAQ, question 4: "Pay yearly and save 20 %"' }],
  items_dropped: 0,
  line: 'Checked: "Add the yearly discount to the FAQ" checked.',
  parts_checked: 1,
  parts_total: 1,
  parts_written: true,
  parts: [{ text: 'Add the yearly discount to the FAQ', result: 'passed', criteria: [{ id: 'c1', text: 'The FAQ mentions the yearly discount', result: 'passed' }] }],
};

/** parts_written false: each criterion stands in as a part, so the card stays today's card. */
export const PARTS_NOT_WRITTEN: Checked = {
  ...CHECKED_PARTLY,
  parts_checked: 1,
  parts_total: 2,
  parts_written: false,
  parts: [
    { text: 'The three plan prices match the plan sheet', result: 'passed', criteria: [{ id: 'c1', text: 'The three plan prices match the plan sheet', result: 'passed' }] },
    { text: 'The FAQ mentions the yearly discount', result: 'not_checked', criteria: [{ id: 'c2', text: 'The FAQ mentions the yearly discount', result: 'not_checked' }] },
  ],
};
