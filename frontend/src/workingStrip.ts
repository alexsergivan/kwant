// The slim "Bot is <step> · 0:42" strip above the composer. Pure helpers; the component only draws them.
import { ActivityStep, LiveActivity, isOpenStep, plainTitle } from './activity';

export interface WorkingState {
  live?: LiveActivity | null;
  queued?: boolean;
  awaitingAnswer?: boolean;
  awaitingBrowser?: boolean;
  /** The reply's first words have started streaming. */
  writing?: boolean;
}

const lower = (t: string) => (t ? t.charAt(0).toLowerCase() + t.slice(1) : t);

/** What the bot is doing right now, in a few lowercase words that follow "<Bot> is". */
export function workingLabel(s: WorkingState): string {
  if (s.queued) return 'waiting in line';
  if (s.awaitingAnswer) return s.awaitingBrowser ? 'waiting for you in the browser' : 'waiting for your answer';
  const open: ActivityStep | undefined = [...(s.live?.steps ?? [])].reverse().find(isOpenStep);
  if (open) {
    const t = plainTitle(open.title).trim();
    if (t) return lower(t);
  }
  if (s.writing) return 'writing the reply';
  return 'getting started';
}

/** "0:42", "12:05", "1:02:09". Negative or non-finite input reads 0:00. */
export function formatElapsed(ms: number): string {
  const total = Number.isFinite(ms) && ms > 0 ? Math.floor(ms / 1000) : 0;
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const sec = String(total % 60).padStart(2, '0');
  return h > 0 ? `${h}:${String(m).padStart(2, '0')}:${sec}` : `${m}:${sec}`;
}

/** When the turn began: the earliest step start, else the moment the strip appeared. */
export function turnStart(live: LiveActivity | null | undefined, mountedAt: number): number {
  let start = mountedAt;
  for (const s of live?.steps ?? []) {
    const t = Date.parse(s.started_at);
    if (Number.isFinite(t) && t < start) start = t;
  }
  return start;
}
