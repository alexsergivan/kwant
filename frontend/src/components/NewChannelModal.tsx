import React, { useEffect, useId, useRef, useState } from 'react';
import { AlertCircle, Hash, X } from 'lucide-react';
import { ModernAvatar } from './ModernAvatar';
import type { Channel, Kwant } from './Sidebar';
import { playSfx } from '../audio/chiptune';
import { channelNameError, channelSlug, newChannelBody, routingHint } from '../channels';

export interface NewChannelModalProps {
  isOpen: boolean;
  onClose: () => void;
  kwants: Kwant[];
  channels: Channel[];
  /** Creates the channel; resolves to an error message, or null once it exists. */
  onCreate: (body: ReturnType<typeof newChannelBody>) => Promise<string | null>;
}

const input =
  'w-full bg-zinc-950 border border-zinc-800 rounded-lg px-3 py-2 text-xs text-zinc-100 placeholder-zinc-600 focus:border-sky-500 focus:outline-none transition-colors';

export const NewChannelModal: React.FC<NewChannelModalProps> = ({ isOpen, onClose, kwants, channels, onCreate }) => {
  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [members, setMembers] = useState<string[]>([]);
  const [touched, setTouched] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const nameRef = useRef<HTMLInputElement>(null);
  const titleId = useId();
  const nameId = useId();
  const hintId = useId();

  useEffect(() => {
    if (!isOpen) return;
    setName('');
    setDescription('');
    // Every bot to start with, so Laya has someone to choose from
    setMembers(kwants.map((k) => k.id));
    setTouched(false);
    setError(null);
    setSubmitting(false);
    requestAnimationFrame(() => nameRef.current?.focus());
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isOpen]);

  useEffect(() => {
    if (!isOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !submitting) onClose();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [isOpen, submitting, onClose]);

  if (!isOpen) return null;

  const slug = channelSlug(name);
  const nameError = channelNameError(slug, channels);
  const showNameError = touched && !!nameError;

  const toggle = (id: string) =>
    setMembers((prev) => (prev.includes(id) ? prev.filter((m) => m !== id) : [...prev, id]));

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    setTouched(true);
    if (nameError || members.length === 0 || submitting) return;
    setSubmitting(true);
    setError(null);
    const failed = await onCreate(newChannelBody(slug, description, members));
    setSubmitting(false);
    if (failed) setError(failed);
  };

  return (
    <div className="fixed inset-0 z-50 bg-black/70 backdrop-blur-sm flex items-center justify-center p-4" onMouseDown={(e) => e.target === e.currentTarget && !submitting && onClose()}>
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        className="bg-zinc-900 border border-zinc-800 rounded-2xl max-w-md w-full max-h-[calc(100dvh-2rem)] overflow-y-auto p-5 sm:p-6 text-zinc-100 shadow-2xl space-y-5"
      >
        <div className="flex items-center justify-between border-b border-zinc-800 pb-3">
          <div className="flex items-center gap-2">
            <div className="w-7 h-7 rounded-lg bg-sky-500/10 border border-sky-500/20 flex items-center justify-center text-sky-400">
              <Hash size={15} />
            </div>
            <h3 id={titleId} className="text-sm font-semibold text-zinc-100">
              New channel
            </h3>
          </div>
          <button
            type="button"
            aria-label="Close"
            onClick={() => {
              playSfx('click');
              onClose();
            }}
            className="p-1 rounded-lg text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800 transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
          >
            <X size={17} />
          </button>
        </div>

        {error && (
          <div role="alert" className="p-2.5 rounded-lg bg-rose-500/10 border border-rose-500/20 flex items-center gap-2 text-rose-400 text-xs">
            <AlertCircle size={15} className="flex-shrink-0" />
            <span>{error}</span>
          </div>
        )}

        {kwants.length === 0 ? (
          <p className="text-xs text-zinc-400">Add a bot to this space first. A channel needs at least one bot to answer.</p>
        ) : (
          <form onSubmit={submit} className="space-y-4" noValidate>
            <div>
              <label htmlFor={nameId} className="block text-xs font-medium text-zinc-400 mb-1">
                Name
              </label>
              <div className="relative">
                <Hash size={13} className="absolute left-3 top-1/2 -translate-y-1/2 text-zinc-500" aria-hidden />
                <input
                  id={nameId}
                  ref={nameRef}
                  type="text"
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  onBlur={() => name && setTouched(true)}
                  placeholder="e.g. launch-plan"
                  autoComplete="off"
                  aria-invalid={showNameError}
                  aria-describedby={showNameError ? `${nameId}-err` : undefined}
                  className={`${input} pl-8 ${showNameError ? 'border-rose-500/60' : ''}`}
                />
              </div>
              {showNameError ? (
                <p id={`${nameId}-err`} className="text-[11px] text-rose-400 mt-1">
                  {nameError}
                </p>
              ) : (
                slug &&
                slug !== name.trim() && (
                  <p className="text-[11px] text-zinc-500 mt-1">
                    Saved as <span className="font-mono text-sky-400">#{slug}</span>
                  </p>
                )
              )}
            </div>

            <div>
              <label className="block text-xs font-medium text-zinc-400 mb-1">
                What it's for <span className="text-zinc-600 font-normal">(optional)</span>
                <input
                  type="text"
                  value={description}
                  onChange={(e) => setDescription(e.target.value)}
                  placeholder="e.g. Planning the spring launch"
                  className={`${input} mt-1 font-normal`}
                />
              </label>
            </div>

            <fieldset aria-describedby={hintId}>
              <div className="flex items-center justify-between mb-1.5">
                <legend className="text-xs font-medium text-zinc-400">Bots in this channel</legend>
                <span className="text-[11px] font-mono text-zinc-500">{members.length} picked</span>
              </div>
              <div className="space-y-1 max-h-52 overflow-y-auto pr-1">
                {kwants.map((k) => {
                  const on = members.includes(k.id);
                  return (
                    <label
                      key={k.id}
                      className={`flex items-center gap-2.5 p-2 rounded-lg border text-xs cursor-pointer transition-all focus-within:outline focus-within:outline-2 focus-within:outline-sky-500 ${
                        on ? 'border-sky-500/40 bg-sky-500/10 text-zinc-100' : 'border-zinc-800 bg-zinc-950/60 text-zinc-400 hover:border-zinc-700'
                      }`}
                    >
                      <input
                        type="checkbox"
                        checked={on}
                        onChange={() => toggle(k.id)}
                        className="rounded border-zinc-700 bg-zinc-900 text-sky-500 focus:ring-0 focus:ring-offset-0"
                      />
                      <ModernAvatar role={k.avatar_sprite} name={k.name} size={24} />
                      <span className="truncate">
                        <span className="font-semibold text-zinc-200">{k.name}</span>
                        <span className="text-[10px] text-zinc-400 ml-1.5">{k.title}</span>
                      </span>
                    </label>
                  );
                })}
              </div>
              <p id={hintId} className={`text-[11px] mt-1.5 ${members.length === 0 && touched ? 'text-rose-400' : 'text-zinc-500'}`}>
                {routingHint(members.length)}
              </p>
            </fieldset>

            <div className="flex items-center justify-end gap-2.5 pt-2 border-t border-zinc-800">
              <button
                type="button"
                onClick={() => {
                  playSfx('click');
                  onClose();
                }}
                className="px-3.5 py-2 rounded-lg text-xs font-medium text-zinc-300 hover:text-zinc-100 hover:bg-zinc-800 transition-colors"
              >
                Cancel
              </button>
              <button
                type="submit"
                disabled={submitting || members.length === 0}
                className="px-4 py-2 rounded-lg text-xs font-medium text-white bg-sky-600 hover:bg-sky-500 transition-colors shadow-sm disabled:opacity-50 disabled:cursor-not-allowed flex items-center gap-1.5"
              >
                <Hash size={14} />
                <span>{submitting ? 'Creating…' : 'Create channel'}</span>
              </button>
            </div>
          </form>
        )}
      </div>
    </div>
  );
};
