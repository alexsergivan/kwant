// #1301: a render crash anywhere in the app showed a blank white page until a refresh. This catches it
// and shows one plain line with Reload instead. `resetKey` (the open space) gives a fresh try when it
// changes, so switching spaces from the rail recovers without a reload.
import React from 'react';
import { AlertTriangle, RotateCw } from 'lucide-react';

export const APP_CRASH_LINE = 'Something went wrong on this page.';

interface Props {
  children: React.ReactNode;
  resetKey?: string | null;
  /** Fills the whole window (outside the shell); otherwise it fills the pane it replaces. */
  full?: boolean;
}

export class AppErrorBoundary extends React.Component<Props, { failed: boolean; key?: string | null }> {
  state: { failed: boolean; key?: string | null } = { failed: false, key: this.props.resetKey };

  static getDerivedStateFromError() {
    return { failed: true };
  }

  static getDerivedStateFromProps(props: Props, state: { failed: boolean; key?: string | null }) {
    return props.resetKey !== state.key ? { failed: false, key: props.resetKey } : null;
  }

  componentDidCatch(error: unknown, info: React.ErrorInfo) {
    console.error('kwant hit an error while showing this page.', error, info?.componentStack ?? '');
  }

  render() {
    if (!this.state.failed) return this.props.children;
    return (
      <div
        role="alert"
        data-app-crash
        className={`${this.props.full ? 'fixed inset-0' : 'flex-1 min-w-0 h-full'} flex items-center justify-center bg-zinc-950 p-6 text-xs text-zinc-300`}
      >
        <div className="flex flex-col items-center gap-3 text-center">
          <p className="flex items-center gap-2">
            <AlertTriangle size={14} className="shrink-0 text-amber-300" aria-hidden="true" />
            {APP_CRASH_LINE}
          </p>
          <button
            type="button"
            onClick={() => window.location.reload()}
            className="kw-btn-primary kw-focus inline-flex items-center gap-1.5 px-3 py-1.5 min-h-[36px] max-sm:min-h-[44px] rounded-lg text-xs"
          >
            <RotateCw size={13} aria-hidden="true" />
            Reload
          </button>
        </div>
      </div>
    );
  }
}
