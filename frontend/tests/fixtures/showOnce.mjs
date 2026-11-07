// Show it once fixtures in api.md's shapes: Rook's "export last week's orders" run (design s-ui.js).
export const LIMITS = { steps: 200, secs: 1800, near_steps_left: 8, near_secs: 1680 };

export const rec = (over = {}) => ({
  id: 'rec_1', kwant_id: 'k_rook', space_id: 'sp1', stage: 'k_rook', state: 'recording',
  started_at: '2026-11-07T10:00:00Z', ended_at: null, elapsed_secs: 168, step_count: 14, note_count: 1,
  limits: LIMITS, near: false, stop_reason: null, origins: ['https://shop.example.com', 'https://crm.example.com'],
  engine: 'claude', write_error: null, skill_id: null, steps_expire_at: null, steps_deleted_at: null, ...over,
});

const at = (i) => `2026-11-07T10:0${Math.floor(i / 6)}:${String((i * 10) % 60).padStart(2, '0')}Z`;
export const STEPS = [
  { kind: 'page', origin: 'https://shop.example.com', path: '/admin/login' },
  { kind: 'type', label: 'Email', value: 'ops@example.com' },
  { kind: 'type', label: 'Password', masked: 'password', placeholder: '[your password: use the saved login]' },
  { kind: 'type', label: 'Code', masked: 'code', placeholder: '[code]' },
  { kind: 'click', role: 'button', name: 'Sign in' },
  { kind: 'page', origin: 'https://shop.example.com', path: '/admin/orders' },
  { kind: 'select', label: 'Date range', value: 'Last 7 days' },
  { kind: 'click', role: 'button', name: 'Export CSV' },
  { kind: 'note', value: 'Only paid orders. Skip refunds.' },
  { kind: 'page', origin: 'https://crm.example.com', path: '/imports/new' },
  { kind: 'upload', label: 'Orders file' },
  { kind: 'select', label: 'Import type', value: 'Orders' },
  { kind: 'click', role: 'button', name: 'Import' },
  { kind: 'key', value: 'Enter' },
  { kind: 'click', role: 'link', name: 'View import' },
].map((s, i) => ({ id: i + 1, n: i + 1, at: at(i), masked: null, placeholder: null, ...s }));

export const SKILL_BODY = `# export-weekly-orders

## Goal
Export last week's paid orders from the shop and import them into the CRM.

## When to use
Every Monday, or when someone asks for last week's orders in the CRM.

## Inputs
| Name | Example from your run |
| --- | --- |
| \`date_range\` | Last 7 days |
| \`import_type\` | Orders |

## Steps
1. Open shop.example.com/admin/login and sign in with \`ops@example.com\` and [your password: use the saved login].
2. Enter [code] when it asks.
3. Open Orders, set Date range to \`date_range\` and click Export CSV.
4. Keep only paid orders. Skip refunds.
5. Open crm.example.com/imports/new, upload the file, choose \`import_type\` and click Import.

## Checks
The import page says the import finished and the row count matches the export.

## What can go wrong
The code expires after a minute. Ask for a new one.

## Sites
shop.example.com, crm.example.com
`;

export const shown = (over = {}) => ({
  recording_id: 'rec_1', shown_at: '2026-11-07T10:04:00Z', shown_on: 'Nov 7, 2026', step_count: 14,
  inputs: [
    { name: 'date_range', description: 'Which orders to export', example: 'Last 7 days' },
    { name: 'import_type', example: 'Orders' },
  ],
  steps_expire_at: '2026-11-14T10:04:00Z', steps_deleted: false, ...over,
});

export const skill = (over = {}) => ({
  id: 'sk_1', kwant_id: 'k_rook', name: 'export-weekly-orders', description: "Export last week's paid orders into the CRM",
  body: SKILL_BODY, status: 'pending', source: 'shown', proposed_by: 'k_rook', replaces_id: null,
  created_at: '2026-11-07T10:05:00Z', updated_at: '2026-11-07T10:05:00Z', reviewed_at: null, shown: shown(), ...over,
});
