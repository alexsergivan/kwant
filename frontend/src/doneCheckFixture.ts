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
