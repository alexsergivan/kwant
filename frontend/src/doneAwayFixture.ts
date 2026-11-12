// 0.9.5 F demo data: mockup 2 ("While you were away") as the API sends it with Vega's part B (#543: unread,
// 7 days kept, the done fields, a team_job item), for tests and the screenshots. Not imported by the app.
// Names, shops and prices are the mockup's made-up ones.

import type { InboxItem, InboxResponse, ReviewItem } from './api/inbox';
import type { Routine, RoutineRun } from './continuity';
import type { DoneApi, DoneDetailData, MessageTarget } from './components/DoneDetail';
import type { DoneMessage } from './doneAway';

/** UTC; the tests and shots run in UTC so the clock reads like the mockup. */
const at = (hms: string, day = '2026-11-11') => `${day}T${hms}Z`;
const NOW = at('00:50:00', '2026-11-12');

export const AWAY_CHANNEL = { id: 'c-spring', name: 'team-Spring launch' };

const approval: InboxItem = {
  kind: 'approval',
  id: 'ap-1',
  kwant_id: 'k-ann',
  kwant_name: 'Ann',
  title: 'Approve “Spring sale: 15% off”?',
  question: 'Approve “Spring sale: 15% off”?',
  summary: 'needs your OK',
  status: 'pending',
  timestamp: at('22:58:00'),
  link: { thread_id: null, channel_id: AWAY_CHANNEL.id, direct_kwant_id: null },
  tool_name: 'gmail',
  details: { to: 'list@beanline.example.com', subject: 'Spring sale: 15% off' },
  group: 'waiting_on_you',
  sort_key: [1, 0, 0, 'Ann', 'ap-1'],
  unread: true,
};

/** The review task the routine's run made (`Routine: <name>`); nothing to check, nothing changed. */
export const AWAY_ROUTINE_REVIEW: ReviewItem = {
  kind: 'review',
  id: 'r-prices',
  kwant_id: 'k-ben',
  kwant_name: 'Ben',
  title: 'Routine: Competitor prices',
  summary: 'No change since last week. Beanline still cheapest.',
  status: 'review',
  timestamp: at('22:51:10'),
  link: { thread_id: null, channel_id: AWAY_CHANNEL.id, direct_kwant_id: null },
  group: 'to_review',
  sort_key: [3, 0, 0, 'Ben', 'r-prices'],
  task_id: 't-prices',
  mission_id: null,
  project: null,
  in_review_since: at('22:51:10'),
  round: { n: 1, max: 2 },
  verdict: {
    id: null,
    result: 'none',
    confidence: null,
    line: '',
    checker: null,
    checked_at: null,
    steps_run: 0,
    steps_total: 0,
  },
  risk: { level: 'low', reasons: [] },
  criteria: [],
  criteria_summary: { met: 0, total: 0, not_met: 0, not_checked: 0 },
  couldnt_verify: [],
  decisions: { total: 0, top: [] },
  evidence: { checks: [], screenshots: [], pr: null },
  held_actions: [],
  previous_round: null,
  stale: null,
  approve_error: null,
  one_tap: false,
  batchable: false,
  rollback_ok: true,
  changed: false,
  unread: true,
  result_message_id: 'm-prices',
  result_conversation_id: AWAY_CHANNEL.id,
  result_thread_id: null,
  result_link: { thread_id: null, channel_id: AWAY_CHANNEL.id, direct_kwant_id: null },
  answer_excerpt: 'No change since last week. Beanline is still the cheapest at $24.00 per kg.',
  trail: [
    { at: at('22:51:02'), kind: 'page', label: 'Opened beanline.example.com', detail: 'found House Blend 1 kg $24.00, 15% off until Mar 31', status: 'ok', by: null },
    { at: at('22:51:05'), kind: 'page', label: 'Opened roastco.example.com', detail: 'found 1 kg $27.00, free shipping over $40', status: 'ok', by: null },
    { at: at('22:51:09'), kind: 'tool', label: 'Compared with last Monday’s run', detail: 'nothing changed', status: 'ok', by: null },
  ],
  trail_dropped: 0,
  why: 'Check Beanline and RoastCo prices and tell me what changed since last week.',
  why_kind: 'routine',
  source: { kind: 'routine', routine_id: 'rt-prices', run_id: 'run-1', trigger: 'manual', task_id: 't-prices' },
} as unknown as ReviewItem;

/** The run's own row (folded into the review above). */
const runRow: InboxItem = {
  kind: 'routine_run',
  id: 'run-1',
  kwant_id: 'k-ben',
  kwant_name: 'Ben',
  title: 'Competitor prices',
  summary: 'No change since last week. Beanline still cheapest.',
  status: 'ok',
  timestamp: at('22:51:10'),
  link: { thread_id: null, channel_id: null, direct_kwant_id: 'k-ben' },
  group: 'finished',
  sort_key: [4, 0, 0, 'Ben', 'run-1'],
  unread: true,
};

/** A lead's `Done:` reply after a team job (part B `team_job`): Ann led Ben and Cody. */
export const AWAY_TEAM_JOB: InboxItem = {
  kind: 'team_job',
  id: 'tj-spring',
  kwant_id: 'k-ann',
  kwant_name: 'Ann',
  lead: { kwant_id: 'k-ann', kwant_name: 'Ann' },
  members: [
    { kwant_id: 'k-ben', kwant_name: 'Ben' },
    { kwant_id: 'k-cody', kwant_name: 'Cody' },
  ],
  title: 'Spring sale plan',
  summary: 'Ben checked 2 shops, Cody drafted the email, you picked 15%',
  status: 'completed',
  timestamp: at('22:45:00'),
  link: { thread_id: null, channel_id: AWAY_CHANNEL.id, direct_kwant_id: null },
  group: 'finished',
  sort_key: [4, 0, 1, 'Ann', 'tj-spring'],
  unread: false,
  changed: true,
  result_message_id: 'm-spring',
  result_conversation_id: AWAY_CHANNEL.id,
  result_thread_id: null,
  result_link: { thread_id: null, channel_id: AWAY_CHANNEL.id, direct_kwant_id: null },
  answer_excerpt: 'Done: Ben checked 2 shops, Cody drafted the sale email, and you picked 15% off. The draft is in the channel.',
  trail: [
    { at: at('22:31:04'), kind: 'page', label: 'Opened beanline.example.com', detail: 'House Blend 1 kg $24.00', status: 'ok', by: 'Ben' },
    { at: at('22:33:40'), kind: 'page', label: 'Opened roastco.example.com', detail: null, status: 'error', by: 'Ben' },
    { at: at('22:38:12'), kind: 'write', label: 'Drafted the sale email', detail: 'Spring sale: 15% off', status: 'ok', by: 'Cody' },
    { at: at('22:41:30'), kind: 'message', label: 'Sent the draft to the list', detail: null, status: 'skipped', by: 'Cody' },
  ],
  trail_dropped: 3,
  why: 'Plan a spring sale for the house blend and draft the email.',
  why_kind: 'request',
  source: { kind: 'team_job', delegation_id: 'dl-1' },
};

export const AWAY_INBOX: InboxResponse = {
  now: NOW,
  since: at('00:50:00', '2026-11-05'),
  last_seen: at('22:50:00'),
  counts: {
    waiting_on_you: 1,
    failed_or_stuck: 0,
    to_review: 1,
    finished: 2,
    badge: 2,
  },
  items: [approval, AWAY_ROUTINE_REVIEW, runRow, AWAY_TEAM_JOB],
  batch: null,
} as InboxResponse;

export const AWAY_ROUTINE: Routine = {
  id: 'rt-prices',
  kwant_id: 'k-ben',
  channel_id: AWAY_CHANNEL.id,
  name: 'Competitor prices',
  instructions: 'Check Beanline and RoastCo prices and tell me what changed since last week.',
  schedule: { type: 'weekly', weekdays: ['mon'], time: '09:00', tz: 'UTC' },
  enabled: true,
  next_run_at: at('09:00:00', '2026-11-16'),
  last_run_at: at('22:51:00'),
  last_status: 'ok',
  created_at: at('10:12:00'),
  thread_id: null,
};

export const AWAY_RUN: RoutineRun = {
  id: 'run-1',
  routine_id: 'rt-prices',
  trigger: 'manual',
  started_at: at('22:51:00'),
  ended_at: at('22:51:10'),
  status: 'ok',
  error: null,
  message_id: 'm-prices',
};

/** A finished hand-off with no review task and none of part B's fields (an older server). */
export const AWAY_HANDOFF: InboxItem = {
  kind: 'delegation',
  id: 'd-spring',
  kwant_id: 'k-cody',
  kwant_name: 'Cody',
  title: 'Spring sale email',
  summary: 'Drafted the sale email for the list.',
  status: 'completed',
  timestamp: at('22:45:00'),
  link: { thread_id: null, channel_id: AWAY_CHANNEL.id, direct_kwant_id: null },
  group: 'finished',
  sort_key: [4, 0, 2, 'Cody', 'd-spring'],
};

const step = (id: string, t: string, title: string, detail: string, kind = 'tool') => ({
  id,
  kind,
  status: 'ok',
  title,
  detail,
  started_at: at(t),
  ended_at: at(t),
});

export const AWAY_MESSAGE: DoneMessage = {
  id: 'm-prices',
  created_at: at('22:51:10'),
  text: [
    'No change since last week. Beanline is still the cheapest at $24.00 per kg.',
    '',
    '| Shop | 1 kg house blend | 250 g | Offer | vs last week |',
    '|---|---|---|---|---|',
    '| Beanline | $24.00 | $13.50 | 15% off until Mar 31 | no change |',
    '| RoastCo | $27.00 | $15.00 | Free shipping over $40 | no change |',
  ].join('\n'),
  meta: {
    activity: {
      steps: [
        {
          id: 's0',
          kind: 'thinking',
          status: 'done',
          title: 'Thinking',
          started_at: at('22:51:01'),
        },
        step('s1', '22:51:02', 'Opened beanline.example.com', 'found House Blend 1 kg $24.00, 15% off until Mar 31'),
        step('s2', '22:51:05', 'Opened roastco.example.com', 'found 1 kg $27.00, free shipping over $40'),
        step('s3', '22:51:09', 'Compared with last Monday’s run', 'nothing changed'),
      ],
      dropped: 0,
    },
  },
};

export const AWAY_TARGET: MessageTarget = {
  message_id: 'm-prices',
  channel_id: AWAY_CHANNEL.id,
  kwant_id: 'k-ben',
  thread_id: null,
};

/** What the pane loads for the routine review, already loaded. */
export const AWAY_DETAIL: DoneDetailData = {
  loading: false,
  error: null,
  routine: AWAY_ROUTINE,
  run: AWAY_RUN,
  message: AWAY_MESSAGE,
  target: AWAY_TARGET,
};

/** A DoneApi over the data above, recording its calls. */
export function awayDoneApi(calls: string[] = []): DoneApi {
  return {
    routines: async () => (calls.push('routines'), [AWAY_ROUTINE]),
    runs: async (id) => (calls.push(`runs:${id}`), [AWAY_RUN]),
    message: async (t) => (calls.push(`message:${t.message_id}`), t.message_id === AWAY_MESSAGE.id ? AWAY_MESSAGE : null),
  };
}

export const awayPlaceName = (id: string | null): string | null => (id === AWAY_CHANNEL.id ? '@Spring launch' : null);
