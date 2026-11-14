// 0.9.8 browser panel (design browser-panel/design.md D): when the panel closes itself.

export interface PaneOpener {
  /** `bot`: a browser event opened it; `person`: a chip, the Bot browser button, a step, a link. */
  by: 'bot' | 'person';
  /** The browser's owner (a bot, team or channel id) whose turn opened it. */
  owner: string;
}

/** After the owner's turn ends, this long for the final step to land before the panel closes. */
export const AUTO_CLOSE_DELAY_MS = 600;
/** The close animation (opacity and a short slide), instant under reduced motion. */
export const PANE_EXIT_MS = 160;

/**
 * Whether the panel closes itself now. Only a panel a bot opened closes, once that bot's turn has
 * ended, and never while the person is using it: touched, in control, a step waiting on them, or
 * Point picks still pending. A panel the person opened never closes itself; it shows Finished.
 */
export function shouldAutoClose(s: { by: PaneOpener['by'] | null; touched: boolean; control: boolean; humanStep: boolean; pointPending: boolean; running: boolean }): boolean {
  return s.by === 'bot' && !s.running && !s.touched && !s.control && !s.humanStep && !s.pointPending;
}

/** Phone width: the panel is a bottom sheet there and never opens by itself. */
export const PHONE_PANE_QUERY = '(max-width: 639px)';

/** Whether a browser event may open the panel: never on a phone (the live chip is the way in), and not when it's open. */
export function shouldAutoOpen(s: { open: boolean; narrow: boolean }): boolean {
  return !s.open && !s.narrow;
}

/** The panel header's status line: "Browsing now", "Finished 10:46 PM", or "Idle". */
export function paneStatus(s: { running: boolean; finishedAt: string | null; locale?: string }): { kind: 'browsing' | 'finished' | 'idle'; text: string } {
  if (s.running) return { kind: 'browsing', text: 'Browsing now' };
  if (s.finishedAt) {
    const d = new Date(s.finishedAt);
    if (!Number.isNaN(d.getTime())) return { kind: 'finished', text: `Finished ${d.toLocaleTimeString(s.locale ?? [], { hour: 'numeric', minute: '2-digit' })}` };
    return { kind: 'finished', text: 'Finished' };
  }
  return { kind: 'idle', text: 'Idle' };
}
