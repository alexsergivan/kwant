import React, { useState } from 'react';
import { ExternalLink, Key, LogIn } from 'lucide-react';
import { EngineInfo, INSTALL_URL } from '../engines';
import { playSfx } from '../audio/chiptune';
import { ClaudeSignIn } from './ClaudeSignIn';
import { NewKeyAccountForm } from './EngineAccounts';

export type StartChoice = 'claude' | 'codex' | 'key';

const KEY_PROVIDERS = [
  { card: 'anthropic_api', label: 'Anthropic' },
  { card: 'openai_api', label: 'OpenAI' },
  { card: 'xai_api', label: 'xAI' },
] as const;

const CHOICES: Array<{ id: StartChoice; title: string; hint: string; Icon: typeof Key }> = [
  { id: 'claude', title: 'Sign in with Claude', hint: 'Uses your Claude Pro or Max plan', Icon: LogIn },
  { id: 'codex', title: 'Sign in with ChatGPT', hint: 'Through Codex, with your ChatGPT plan', Icon: LogIn },
  { id: 'key', title: 'Paste an API key', hint: 'Anthropic, OpenAI or xAI', Icon: Key },
];

/** Inline `code` in an engine's hint like "Not signed in: run `codex login`". */
const withCode = (text: string) =>
  text.split(/(`[^`]+`)/).map((part, i) =>
    part.startsWith('`') && part.endsWith('`') ? (
      <code key={i} className="font-mono text-[10.5px] px-1 py-px rounded bg-zinc-800 text-zinc-200 select-text whitespace-nowrap">
        {part.slice(1, -1)}
      </code>
    ) : (
      <React.Fragment key={i}>{part}</React.Fragment>
    ),
  );

const InstallLink: React.FC<{ engine: EngineInfo | undefined; name: string }> = ({ engine, name }) => {
  const url = engine ? INSTALL_URL[engine.id] : undefined;
  return (
    <p className="text-[11px] text-zinc-400 leading-snug">
      {name} isn’t on this computer yet.{' '}
      {url && (
        <a href={url} target="_blank" rel="noreferrer" className="inline-flex items-center gap-0.5 text-violet-300 hover:text-violet-200 underline-offset-2 hover:underline">
          Install {name} <ExternalLink size={10} aria-hidden="true" />
        </a>
      )}
      {url && ', then press Re-check.'}
    </p>
  );
};

/** Settings when nothing can run yet: three ways to start, one opened at a time. */
export const StartPicker: React.FC<{
  engines: EngineInfo[];
  onSignedIn: () => void;
  onKeyAdded: () => void;
  focusClaudeSignIn?: number;
  initial?: StartChoice | null;
}> = ({ engines, onSignedIn, onKeyAdded, focusClaudeSignIn, initial = null }) => {
  const [choice, setChoice] = useState<StartChoice | null>(initial ?? (focusClaudeSignIn ? 'claude' : null));
  const [keyCard, setKeyCard] = useState<string>('anthropic_api');
  const claude = engines.find((e) => e.id === 'claude');
  const codex = engines.find((e) => e.id === 'codex');

  const body = () => {
    if (choice === 'claude') {
      if (!claude || claude.status === 'not_installed') return <InstallLink engine={claude ?? ({ id: 'claude' } as EngineInfo)} name="Claude Code" />;
      if (claude.enabled === false) return <p className="text-[11px] text-zinc-400">Claude is switched off. Turn it on in its card below.</p>;
      return <ClaudeSignIn onSignedIn={onSignedIn} focusRequest={focusClaudeSignIn} />;
    }
    if (choice === 'codex') {
      if (!codex || codex.status === 'not_installed') return <InstallLink engine={codex ?? ({ id: 'codex' } as EngineInfo)} name="Codex" />;
      if (codex.enabled === false) return <p className="text-[11px] text-zinc-400">Codex is switched off. Turn it on in its card below.</p>;
      return (
        <p className="text-[11px] text-zinc-400 leading-snug">
          {withCode(codex.detail || 'Run `codex login` on this computer and pick “Sign in with ChatGPT”.')} Then press Re-check.
        </p>
      );
    }
    if (choice === 'key')
      return (
        <div className="space-y-2">
          <div role="radiogroup" aria-label="Key from" className="flex flex-wrap gap-1.5">
            {KEY_PROVIDERS.map((p) => (
              <button
                key={p.card}
                type="button"
                role="radio"
                aria-checked={keyCard === p.card}
                onClick={() => setKeyCard(p.card)}
                className={`px-2.5 py-1 rounded-full text-[11px] border transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-violet-400 ${
                  keyCard === p.card ? 'border-violet-500/60 bg-violet-500/15 text-violet-100' : 'border-zinc-800 text-zinc-400 hover:text-zinc-200'
                }`}
              >
                {p.label}
              </button>
            ))}
          </div>
          <NewKeyAccountForm card={keyCard} onCancel={() => setChoice(null)} onCreated={onKeyAdded} />
        </div>
      );
    return null;
  };

  return (
    <div data-start-picker="" className="rounded-xl border border-violet-500/25 bg-violet-500/[0.05] p-3 space-y-2.5">
      <div>
        <h4 className="text-xs font-semibold text-zinc-100">Pick one to start</h4>
        <p className="text-[11px] text-zinc-400">Your bots answer once one of these works. You can add more later.</p>
      </div>
      <div role="radiogroup" aria-label="How to start" className="grid grid-cols-1 sm:grid-cols-3 gap-2">
        {CHOICES.map(({ id, title, hint, Icon }) => (
          <button
            key={id}
            type="button"
            role="radio"
            aria-checked={choice === id}
            data-start-choice={id}
            onClick={() => {
              playSfx('click');
              setChoice(id);
            }}
            className={`text-left rounded-lg border px-3 py-2.5 min-h-[44px] transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-violet-400 ${
              choice === id ? 'border-violet-500/60 bg-violet-500/10' : 'border-zinc-800 bg-zinc-900/80 hover:border-zinc-700'
            }`}
          >
            <span className="flex items-center gap-1.5 text-xs font-medium text-zinc-100">
              <Icon size={13} className="text-violet-300" aria-hidden="true" />
              {title}
            </span>
            <span className="block mt-0.5 text-[11px] text-zinc-500">{hint}</span>
          </button>
        ))}
      </div>
      {choice && <div className="pt-1">{body()}</div>}
    </div>
  );
};
