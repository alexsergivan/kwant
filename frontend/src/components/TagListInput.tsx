import React, { useId, useState } from 'react';
import { X } from 'lucide-react';
import { MAX_TAGS, addTags } from '../tagList';

/** A list of short phrases: type and press Enter (or a comma) to add one, × or Backspace to remove. */
export const TagListInput: React.FC<{
  label: string;
  hint: string;
  placeholder: string;
  value: string[];
  onChange: (next: string[]) => void;
  tone?: 'good' | 'bad';
}> = ({ label, hint, placeholder, value, onChange, tone = 'good' }) => {
  const id = useId();
  const [draft, setDraft] = useState('');
  const [error, setError] = useState<string | null>(null);
  const chip = tone === 'good' ? 'border-emerald-500/30 bg-emerald-500/10 text-emerald-100' : 'border-amber-500/30 bg-amber-500/10 text-amber-100';

  const commit = () => {
    if (!draft.trim()) return;
    const r = addTags(value, draft);
    setError(r.error);
    if (r.list.length !== value.length) onChange(r.list);
    if (!r.error) setDraft('');
  };

  return (
    <div>
      <label htmlFor={id} className="block text-xs font-medium text-zinc-400 mb-1">
        {label} <span className="text-zinc-600 font-normal">· {hint}</span>
      </label>
      <div className="flex flex-wrap items-center gap-1.5 w-full bg-zinc-950 border border-zinc-800 rounded-lg px-2 py-1.5 focus-within:border-sky-500 transition-colors">
        {value.map((t) => (
          <span key={t} className={`inline-flex items-center gap-1 max-w-full rounded-md border pl-2 pr-0.5 text-[11px] ${chip}`}>
            <span className="truncate">{t}</span>
            <button
              type="button"
              onClick={() => onChange(value.filter((x) => x !== t))}
              aria-label={`Remove ${t}`}
              className="p-1 rounded text-zinc-400 hover:text-zinc-100 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
            >
              <X size={11} aria-hidden="true" />
            </button>
          </span>
        ))}
        <input
          id={id}
          type="text"
          value={draft}
          onChange={(e) => {
            setError(null);
            if (e.target.value.includes(',')) {
              const r = addTags(value, e.target.value);
              setError(r.error);
              onChange(r.list);
              setDraft('');
            } else setDraft(e.target.value);
          }}
          onKeyDown={(e) => {
            if (e.key === 'Enter') {
              e.preventDefault();
              commit();
            } else if (e.key === 'Backspace' && !draft && value.length) onChange(value.slice(0, -1));
          }}
          onBlur={commit}
          placeholder={value.length ? '' : placeholder}
          aria-describedby={error ? `${id}-err` : undefined}
          disabled={value.length >= MAX_TAGS && !draft}
          className="flex-1 min-w-[8rem] bg-transparent py-0.5 text-xs text-zinc-100 placeholder-zinc-600 focus:outline-none"
        />
      </div>
      {error && (
        <p id={`${id}-err`} role="alert" className="mt-1 text-[11px] text-rose-400">
          {error}
        </p>
      )}
    </div>
  );
};
