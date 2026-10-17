import React, { useEffect } from 'react';
import { ArrowRightLeft, X } from 'lucide-react';

export interface Toast {
  id: number;
  title: string;
  detail?: string;
  /** One button, e.g. "Open" a chat the toast is about. */
  action?: { label: string; onClick: () => void };
}

const TOAST_MS = 7000;

const ToastItem: React.FC<{ toast: Toast; onDismiss: (id: number) => void }> = ({ toast, onDismiss }) => {
  useEffect(() => {
    const t = setTimeout(() => onDismiss(toast.id), TOAST_MS);
    return () => clearTimeout(t);
  }, [toast.id, onDismiss]);
  return (
    <div className="kw-toast pointer-events-auto flex items-start gap-2.5 w-full bg-zinc-900/95 border border-zinc-700 rounded-xl px-3 py-2.5 shadow-xl text-xs">
      <ArrowRightLeft size={14} className="mt-0.5 shrink-0 text-amber-300" aria-hidden="true" />
      <div className="flex-1 min-w-0">
        <p className="font-medium text-zinc-100">{toast.title}</p>
        {toast.detail && <p className="mt-0.5 text-zinc-400">{toast.detail}</p>}
        {toast.action && (
          <button
            type="button"
            onClick={() => {
              toast.action?.onClick();
              onDismiss(toast.id);
            }}
            className="mt-1.5 px-2 py-0.5 rounded-md text-[11px] font-medium text-sky-300 bg-sky-500/10 border border-sky-500/30 hover:bg-sky-500/20 transition-colors"
          >
            {toast.action.label}
          </button>
        )}
      </div>
      <button
        type="button"
        onClick={() => onDismiss(toast.id)}
        aria-label="Dismiss"
        className="p-0.5 rounded text-zinc-500 hover:text-zinc-200 hover:bg-zinc-800 transition-colors"
      >
        <X size={13} />
      </button>
    </div>
  );
};

/** Short notices in the corner; screen readers hear them without moving focus. */
export const Toasts: React.FC<{ toasts: Toast[]; onDismiss: (id: number) => void }> = ({ toasts, onDismiss }) => (
  <div
    role="status"
    aria-live="polite"
    className="pointer-events-none fixed bottom-4 right-4 left-4 sm:left-auto z-[60] flex flex-col items-end gap-2 sm:w-80"
  >
    {toasts.map((t) => (
      <ToastItem key={t.id} toast={t} onDismiss={onDismiss} />
    ))}
  </div>
);
