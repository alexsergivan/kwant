// 0.9.0 V1: the Inbox store. One module-level store for Inbox › New: the last GET /api/inbox answer,
// the selected item, where the person came from (the back crumb), the status notice, and the Approve all
// run. The run lives here, not in a component, so it keeps going when the person leaves the page.
//
// Nothing here decides what is safe to approve: `one_tap`, `batchable` and `batch` come from the server.
// The client only ever calls verify with the verdict id the server sent.

import { useSyncExternalStore } from 'react';
import { AwayItem, AwayResponse, AwaySection, awayApi, sortAwaySection } from './away';
import { InboxError, InboxGroup, InboxItem, InboxResponse, ReviewItem, cmpSortKey, inboxApi, isReview } from './api/inbox';

export type Crumb = 'board' | 'dev' | null;

export type BatchRowStatus = 'waiting' | 'approving' | 'approved' | 'came_back' | 'error' | 'stopped';

export interface BatchRow {
  id: string;
  task_id: string;
  title: string;
  kwant_id: string;
  kwant_name: string;
  status: BatchRowStatus;
  /** "README.md changed after its check", or the error. */
  note?: string;
}

export interface BatchRun {
  rows: BatchRow[];
  /** Index being approved; -1 when finished. */
  current: number;
  running: boolean;
  stopRequested: boolean;
  /** Set when a sign-in or network error stopped the run. */
  halted: string | null;
}

export interface InboxNotice {
  text: string;
  tone: 'ok' | 'info' | 'error';
}

export interface InboxState {
  data: InboxResponse | null;
  loading: boolean;
  error: string | null;
  /** The item (or 'batch') open in the pane. */
  selectedId: string | null;
  crumb: Crumb;
  notice: InboxNotice | null;
  /** Item id with an inline Approve or Check again in flight. */
  busy: string | null;
  batch: BatchRun | null;
  /** True when /api/inbox isn't there and the list was built from /api/away. */
  fromAway: boolean;
  /** Window start kept from the first load, so refreshes keep the same list. */
  since: string | null;
  /** An item asked for (`?item=`, or Review on the Mission board) that the list may not have loaded yet. */
  pendingKey: string | null;
}

const initial = (): InboxState => ({
  data: null,
  loading: false,
  error: null,
  selectedId: null,
  crumb: null,
  notice: null,
  busy: null,
  batch: null,
  fromAway: false,
  since: null,
  pendingKey: null,
});

let state: InboxState = initial();
const listeners = new Set<() => void>();

function set(patch: Partial<InboxState> | ((s: InboxState) => Partial<InboxState>)) {
  const p = typeof patch === 'function' ? patch(state) : patch;
  state = { ...state, ...p };
  listeners.forEach((l) => l());
}

export const inboxStore = {
  get: () => state,
  subscribe(l: () => void) {
    listeners.add(l);
    return () => listeners.delete(l);
  },
  /** Merges a change into the state (the page's own question and approval actions use it). */
  patch(p: Partial<InboxState>) {
    set(p);
  },
  /** Tests and screenshots only. */
  reset(over: Partial<InboxState> = {}) {
    state = { ...initial(), ...over };
    listeners.forEach((l) => l());
  },
};

export function useInbox(): InboxState {
  return useSyncExternalStore(inboxStore.subscribe, inboxStore.get, inboxStore.get);
}

// ---- Loading ----

const AWAY_GROUP: Record<AwaySection, InboxGroup> = { waiting: 'waiting_on_you', failed: 'failed_or_stuck', finished: 'finished' };
const GROUP_RANK: Record<InboxGroup, number> = { waiting_on_you: 1, failed_or_stuck: 2, to_review: 3, finished: 4 };

/** Older servers have no /api/inbox: the same three sections from /api/away, in today's order. */
export function inboxFromAway(a: AwayResponse): InboxResponse {
  const items: InboxItem[] = [];
  for (const sec of ['waiting', 'failed', 'finished'] as AwaySection[]) {
    sortAwaySection(sec, a[sec], a.last_seen).forEach((i: AwayItem, n) => {
      const group = AWAY_GROUP[sec];
      items.push({ ...i, group, sort_key: [GROUP_RANK[group], 0, n, i.kwant_name, i.id] });
    });
  }
  return {
    now: a.now,
    since: a.since,
    last_seen: a.last_seen,
    counts: { waiting_on_you: a.waiting.length, failed_or_stuck: a.failed.length, to_review: 0, finished: a.finished.length },
    items,
    batch: null,
  };
}

let loadSeq = 0;
/** Overridable for screenshots and tests. */
export const inboxSource: { get: (since?: string) => Promise<InboxResponse | { away: AwayResponse }> } = {
  async get(since) {
    try {
      return await inboxApi.get(since);
    } catch (e) {
      if (e instanceof InboxError && e.status === 404) return { away: await awayApi.get(since) };
      throw e;
    }
  },
};

/** Loads (or refreshes) the list. Keeps the last good list on screen when it fails. */
export async function loadInbox(): Promise<InboxResponse | null> {
  const seq = ++loadSeq;
  set({ loading: true });
  try {
    const got = await inboxSource.get(state.since ?? undefined);
    if (seq !== loadSeq) return state.data;
    const fromAway = 'away' in got;
    const data = fromAway ? inboxFromAway(got.away) : (got as InboxResponse);
    set((s) => ({ data: withLocal(data, s), loading: false, error: null, fromAway, since: s.since ?? data.since }));
    return state.data;
  } catch (e) {
    if (seq !== loadSeq) return state.data;
    set({ loading: false, error: e instanceof Error ? e.message : 'Something went wrong.' });
    return null;
  }
}

/** Items that came back from a batch stay in Failed or stuck until the server says otherwise. */
const cameBack = new Map<string, InboxItem>();

function withLocal(data: InboxResponse, s: InboxState): InboxResponse {
  let items = data.items;
  for (const [id, item] of cameBack) {
    if (items.some((i) => i.id === id && i.group === 'failed_or_stuck')) {
      cameBack.delete(id);
      continue;
    }
    items = [...items.filter((i) => i.id !== id), item];
  }
  // While a run is going, its items stay out of the queue rows (the card shows them).
  const running = s.batch?.running ? new Set(s.batch.rows.map((r) => r.id)) : null;
  const batchIds = data.batch?.ids ?? [];
  if (running) items = items.filter((i) => !running.has(i.id) || i.group === 'failed_or_stuck');
  return { ...data, items: [...items].sort(cmpSortKey), batch: data.batch && batchIds.length ? data.batch : running ? s.data?.batch ?? null : data.batch };
}

// ---- Selection and crumb ----

export function select(id: string | null) {
  set({ selectedId: id });
}

/**
 * Opens an item by its id or task id, from a deep link or another page. It is selected as soon as the
 * list has it; `crumb` says which page the back crumb returns to.
 */
export function openItem(key: string, crumb: Crumb = null) {
  const found = findItem(state.data, key);
  set({ pendingKey: found ? null : key, selectedId: found ? found.id : state.selectedId, crumb });
}

/** Resolves a pending `openItem` once the list has loaded. Returns true when it is now selected. */
export function resolvePending(): boolean {
  if (!state.pendingKey || !state.data) return false;
  const found = findItem(state.data, state.pendingKey);
  set({ pendingKey: null, selectedId: found ? found.id : state.selectedId });
  return !!found;
}

export function setCrumb(c: Crumb) {
  set({ crumb: c });
}

export function setNotice(n: InboxNotice | null) {
  set({ notice: n });
}

/** The item for `?item=`: its id, or its task id (the Mission board and Dev desk know only the task). */
export function findItem(data: InboxResponse | null, key: string | null): InboxItem | null {
  if (!data || !key) return null;
  const all = [...data.items, ...(data.batch_items ?? [])];
  return all.find((i) => i.id === key) ?? all.find((i) => i.task_id === key) ?? null;
}

/** The review item for a task, for the Mission board card and drawer. */
export function reviewForTask(taskId: string): ReviewItem | null {
  const d = state.data;
  if (!d) return null;
  const i = [...d.items, ...(d.batch_items ?? [])].find((x) => x.task_id === taskId);
  return isReview(i) ? i : null;
}

/** The batch's items, in queue order: the server's list when it sends one, else what the queue has. */
export function batchItems(data: InboxResponse | null): InboxItem[] {
  if (!data?.batch) return [];
  const pool = [...(data.batch_items ?? []), ...data.items];
  return data.batch.ids.map((id) => pool.find((i) => i.id === id)).filter((i): i is InboxItem => !!i);
}

// ---- Approve ----

function staleNote(paths: string[]): string {
  if (!paths.length) return 'Its files changed after its check';
  return `${paths[0]}${paths.length > 1 ? ` and ${paths.length - 1} more` : ''} changed after its check`;
}

/** Puts an item that failed freshness into Failed or stuck, locally, until the next load agrees. */
export function comeBack(item: InboxItem, paths: string[]) {
  const back: InboxItem = {
    ...item,
    group: 'failed_or_stuck',
    sort_key: [2, 0, Date.now(), item.kwant_name, item.id],
    stale: { paths, at: new Date().toISOString() },
    one_tap: false,
    batchable: false,
  };
  cameBack.set(item.id, back);
  set((s) => (s.data ? { data: { ...s.data, items: [...s.data.items.filter((i) => i.id !== item.id), back].sort(cmpSortKey) } } : {}));
}

function dropItem(id: string) {
  set((s) => {
    if (!s.data) return {};
    const items = s.data.items.filter((i) => i.id !== id);
    return { data: { ...s.data, items } };
  });
}

/** The next item after `id` in queue order, to open once `id` leaves. */
export function nextAfter(data: InboxResponse | null, id: string): string | null {
  if (!data) return null;
  const rows = data.items;
  const at = rows.findIndex((i) => i.id === id);
  const after = rows.slice(at + 1).find((i) => isReview(i)) ?? rows.slice(0, Math.max(0, at)).reverse().find((i) => isReview(i));
  return after ? after.id : data.batch ? 'batch' : null;
}

/** Inline Approve on a one-tap row. */
export async function approveOne(item: InboxItem): Promise<void> {
  if (!isReview(item) || state.busy) return;
  const title = item.title;
  set({ busy: item.id, notice: null });
  try {
    await inboxApi.verify(item.task_id, item.verdict.id);
    const next = state.selectedId === item.id ? nextAfter(state.data, item.id) : state.selectedId;
    dropItem(item.id);
    set({ busy: null, selectedId: next, notice: { text: `Approved: “${title}”.`, tone: 'ok' } });
  } catch (e) {
    if (e instanceof InboxError && e.stale) {
      comeBack(item, e.paths);
      set({ busy: null, notice: { text: `Couldn’t approve: ${title}. ${staleNote(e.paths)}, so nothing was approved.`, tone: 'error' } });
    } else {
      const why = e instanceof Error ? e.message : 'Something went wrong';
      set({ busy: null, notice: { text: `Couldn’t approve “${title}”. ${why}. Nothing was approved.`, tone: 'error' } });
    }
  }
  void loadInbox();
}

/** "Check again" on an item that failed freshness: reruns the checks; it goes back to To review when they pass. */
export async function checkAgain(item: InboxItem): Promise<void> {
  if (!item.task_id || state.busy) return;
  set({ busy: item.id, notice: null });
  try {
    await inboxApi.runChecks(item.task_id);
    cameBack.delete(item.id);
    set({ busy: null, notice: { text: `Checking “${item.title}” again. It goes back to To review when the checks pass.`, tone: 'info' } });
  } catch (e) {
    set({ busy: null, notice: { text: `Couldn’t start the checks for “${item.title}”. ${e instanceof Error ? e.message : ''}`.trim(), tone: 'error' } });
  }
  void loadInbox();
}

// ---- Approve all ----

/** Starts Approve all. One verify at a time, in queue order; keeps going if the page unmounts. */
export async function approveAll(): Promise<void> {
  if (state.batch?.running) return;
  const items = batchItems(state.data).filter(isReview);
  if (items.length < 2) return;
  const rows: BatchRow[] = items.map((i) => ({ id: i.id, task_id: i.task_id, title: i.title, kwant_id: i.kwant_id, kwant_name: i.kwant_name, status: 'waiting' }));
  set({ batch: { rows, current: 0, running: true, stopRequested: false, halted: null }, notice: null, selectedId: state.selectedId ?? 'batch' });
  await runBatch(items);
}

/** Finishes the current item, then leaves the rest in To review. */
export function stopAfterThisOne() {
  set((s) => (s.batch?.running ? { batch: { ...s.batch, stopRequested: true } } : {}));
}

function patchRow(idx: number, patch: Partial<BatchRow>, run: Partial<BatchRun> = {}) {
  set((s) => {
    if (!s.batch) return {};
    const rows = s.batch.rows.map((r, i) => (i === idx ? { ...r, ...patch } : r));
    return { batch: { ...s.batch, ...run, rows } };
  });
}

async function runBatch(items: ReviewItem[]) {
  for (let i = 0; i < items.length; i++) {
    const b = state.batch;
    if (!b) return;
    if (b.stopRequested) {
      set((s) => ({ batch: s.batch && { ...s.batch, rows: s.batch.rows.map((r, n) => (n >= i && r.status === 'waiting' ? { ...r, status: 'stopped' } : r)) } }));
      break;
    }
    const item = items[i];
    patchRow(i, { status: 'approving' }, { current: i });
    try {
      await inboxApi.verify(item.task_id, item.verdict.id);
      patchRow(i, { status: 'approved' });
      announce(`Approved: “${item.title}”.`);
    } catch (e) {
      if (e instanceof InboxError && e.stale) {
        comeBack(item, e.paths);
        patchRow(i, { status: 'came_back', note: `${staleNote(e.paths)} · back in Failed or stuck` });
        announce(`“${item.title}” came back: ${staleNote(e.paths)}.`);
        continue;
      }
      const status = e instanceof InboxError ? e.status : 0;
      const why = e instanceof Error ? e.message : 'Something went wrong';
      patchRow(i, { status: 'error', note: `Couldn’t approve: ${why}. Nothing was approved.` });
      if (status === 0 || status === 401 || status === 403) {
        // Every next call would fail the same way: stop, and leave the rest in To review.
        set((s) => ({
          batch: s.batch && {
            ...s.batch,
            halted: status === 0 ? 'The server didn’t answer, so the batch stopped.' : 'You need to sign in again, so the batch stopped.',
            rows: s.batch.rows.map((r, n) => (n > i && r.status === 'waiting' ? { ...r, status: 'stopped' } : r)),
          },
        }));
        break;
      }
    }
  }
  const done = state.batch;
  if (done) {
    const ok = done.rows.filter((r) => r.status === 'approved').length;
    const back = done.rows.filter((r) => r.status === 'came_back').length;
    const left = done.rows.filter((r) => r.status === 'stopped').length;
    const parts = [`Approved ${ok} of ${done.rows.length}`];
    if (back) parts.push(`${back} came back`);
    if (left) parts.push(`${left} left in To review`);
    set({ batch: { ...done, running: false, current: -1, stopRequested: false }, notice: { text: `${parts.join(' · ')}.`, tone: back || done.halted ? 'info' : 'ok' } });
  }
  await loadInbox();
}

function announce(text: string) {
  set({ notice: { text, tone: 'info' } });
}

/** Clears a finished run's rows (the card goes back to whatever the server sends). */
export function clearBatch() {
  if (state.batch?.running) return;
  set({ batch: null });
}


/**
 * How other pages reach the Inbox. App sets `go` (show Inbox › New); while it is unset (tests, a page
 * mounted on its own) the Mission board and Dev desk keep their old review buttons.
 */
export const inboxNav: { go: ((crumb: Crumb) => void) | null } = { go: null };

/** The Mission board's Review button and the Dev desk's Review in Inbox: open the task's item, with a crumb back. */
export function openReview(taskId: string, crumb: Crumb): void {
  openItem(taskId, crumb);
  inboxNav.go?.(crumb);
}

/** The review item whose PR is this one (the Dev desk card knows the PR, not the task). */
export function reviewForPr(url: string | null | undefined, number: number | null | undefined): ReviewItem | null {
  const d = state.data;
  if (!d || (!url && number == null)) return null;
  const norm = (u: string) => u.replace(/\/+$/, '').toLowerCase();
  const all = [...d.items, ...(d.batch_items ?? [])].filter(isReview);
  return all.find((i) => !!url && !!i.evidence.pr?.url && norm(i.evidence.pr.url) === norm(url)) ?? null;
}
