// 0.7.7 T7 entry points: the welcome cards (T7.1), the New team tiles (T7.2) and the sidebar box (T7.3).
// Each opens the same preview sheet. Design: next-077/design/design.md §T7.
import React from 'react';
import { ChevronRight, Sparkles, X } from 'lucide-react';
import { ModernAvatar } from './ModernAvatar';
import { TEMPLATE_ICONS, TemplateGlyph } from './TeamPreviewSheet';
import { useMedia } from '../useMedia';
import { PHONE_QUERY } from '../layout';
import { plural, type TeamTemplate } from '../teams/teamTemplates';

const FOCUS = 'focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-sky-400';

/** T7.1: under FirstRunWelcome. Works before any account exists. */
export const TemplateWelcome: React.FC<{ templates: TeamTemplate[]; onPick: (id: string) => void; onBlank: () => void }> = ({ templates, onPick, onBlank }) => {
  const phone = useMedia(PHONE_QUERY);
  if (!templates.length) return null;
  return (
    <section aria-labelledby="t-welcome-h" className="w-full max-w-[752px] mx-auto mt-8 px-4" data-template-welcome>
      <div className="flex items-center gap-3">
        <span className="flex-1 h-px bg-white/10" aria-hidden="true" />
        <h3 id="t-welcome-h" className="text-[11px] leading-4 font-semibold uppercase tracking-[.08em] text-zinc-400">
          Or start with a team
        </h3>
        <span className="flex-1 h-px bg-white/10" aria-hidden="true" />
      </div>
      <p className="mt-2 mb-3 text-[13px] text-zinc-400 text-center">A lead, helpers, instructions and skills for one job. You can add the account after.</p>
      <ul className={phone ? 'flex flex-col gap-2' : 'grid grid-cols-3 gap-3'}>
        {templates.map((t) => (
          <li key={t.id}>
            <button
              type="button"
              data-template={t.id}
              onClick={() => onPick(t.id)}
              aria-label={`${t.label} team: ${t.card_line} ${plural(t.bots.length, 'bot')}. Preview`}
              className={
                phone
                  ? `w-full flex items-center gap-3 p-3 rounded-xl border border-white/10 bg-zinc-900/60 text-left ${FOCUS}`
                  : `w-full h-full flex flex-col gap-2 p-3.5 min-h-[152px] rounded-xl border border-white/10 bg-zinc-900/60 shadow-sm text-left hover:-translate-y-px hover:border-sky-300/45 transition motion-reduce:transition-none motion-reduce:hover:translate-y-0 ${FOCUS}`
              }
            >
              {phone ? (
                <>
                  <TemplateGlyph icon={t.icon} />
                  <span className="flex-1 min-w-0">
                    <span className="block text-sm leading-5 font-semibold text-zinc-50">{t.label}</span>
                    <span className="block text-[12.5px] leading-[18px] text-zinc-300">{t.card_line}</span>
                  </span>
                  <ChevronRight size={16} className="text-zinc-400 shrink-0" aria-hidden="true" />
                </>
              ) : (
                <>
                  <span className="flex items-center gap-2.5">
                    <TemplateGlyph icon={t.icon} />
                    <span className="text-sm leading-5 font-semibold text-zinc-50">{t.label}</span>
                  </span>
                  <span className="text-[12.5px] leading-[18px] text-zinc-300">{t.card_line}</span>
                  <span className="mt-auto flex items-center gap-2" aria-hidden="true">
                    <span className="flex -space-x-[5px]">
                      {t.bots.map((b) => (
                        <span key={b.key} className="rounded-full ring-2 ring-zinc-900">
                          <ModernAvatar role={b.avatar_sprite} name={b.name} size={20} />
                        </span>
                      ))}
                    </span>
                    <span className="text-xs text-zinc-400">{plural(t.bots.length, 'bot')}</span>
                    <span className="ml-auto text-xs font-medium text-sky-300">Preview ›</span>
                  </span>
                </>
              )}
            </button>
          </li>
        ))}
      </ul>
      <div className="mt-2 text-center">
        <button type="button" onClick={onBlank} className={`${phone ? 'min-h-11' : 'min-h-8'} px-2 rounded-md text-[12.5px] text-zinc-300 hover:text-zinc-100 ${FOCUS}`}>
          Start blank instead
        </button>
      </div>
    </section>
  );
};

/** T7.2: the top of the Create New Team dialog. */
export const TemplateTiles: React.FC<{ templates: TeamTemplate[]; onPick: (id: string) => void }> = ({ templates, onPick }) => {
  if (!templates.length) return null;
  return (
    <div data-template-tiles>
      <p className="block text-sm font-medium text-zinc-300 mb-2">From a template</p>
      <div className="grid grid-cols-3 gap-2">
        {templates.map((t) => (
          <button
            key={t.id}
            type="button"
            data-template={t.id}
            onClick={() => onPick(t.id)}
            aria-label={`${t.label} template, ${plural(t.bots.length, 'bot')}. Preview`}
            className={`flex flex-col items-start gap-1.5 p-2.5 min-h-[76px] rounded-[10px] border border-white/10 bg-zinc-900/60 text-left hover:border-sky-300/45 transition-colors ${FOCUS}`}
          >
            <TemplateGlyph icon={t.icon} size={14} box="w-[26px] h-[26px] rounded-[7px]" />
            <span className="text-[13px] leading-4 font-semibold text-zinc-50">{t.label}</span>
            <span className="text-[11px] leading-[14px] text-zinc-400">{plural(t.bots.length, 'bot')}</span>
          </button>
        ))}
      </div>
      <div className="flex items-center gap-3 my-4" aria-hidden="true">
        <span className="flex-1 h-px bg-white/10" />
        <span className="text-[11px] uppercase tracking-[.08em] text-zinc-500">or start blank</span>
        <span className="flex-1 h-px bg-white/10" />
      </div>
    </div>
  );
};

/** T7.3: a dashed box at the bottom of the Bots list while the space has no teams. */
export const TemplateSidebarBox: React.FC<{ templates: TeamTemplate[]; onPick: (id: string) => void; onHide: () => void; onDescribe?: () => void }> = ({ templates, onPick, onHide, onDescribe }) => {
  const phone = useMedia(PHONE_QUERY);
  if (!templates.length) return null;
  return (
    <div className="relative mx-1 mt-2 p-2.5 rounded-[10px] border border-dashed border-white/10" data-template-sidebar>
      <div className={phone ? 'pr-11' : 'pr-7'}>
        <p className="text-[12.5px] leading-[18px] font-semibold text-zinc-200">Start a ready-made team</p>
        <p className="text-[11px] leading-4 text-zinc-400">Bots, instructions and skills in one step.</p>
      </div>
      <div className="mt-2 flex flex-wrap gap-1.5">
        {onDescribe && (
          <button
            type="button"
            data-describe-own
            onClick={onDescribe}
            aria-label="Describe your own team"
            className={`inline-flex items-center gap-[5px] ${phone ? 'h-11 px-3.5' : 'h-[26px] px-[9px]'} rounded-full border border-sky-300/35 bg-sky-300/[.08] text-xs font-medium text-sky-200 hover:bg-sky-300/[.14] ${FOCUS}`}
          >
            <Sparkles size={12} className="text-sky-300" aria-hidden="true" />
            Describe your own
          </button>
        )}
        {templates.map((t) => {
          const Icon = TEMPLATE_ICONS[t.icon];
          return (
            <button
              key={t.id}
              type="button"
              data-template={t.id}
              onClick={() => onPick(t.id)}
              aria-label={`${t.label} team. Preview`}
              className={`inline-flex items-center gap-[5px] ${phone ? 'h-11 px-3.5' : 'h-[26px] px-[9px]'} rounded-full border border-white/10 bg-zinc-900/60 text-xs font-medium text-zinc-200 hover:border-sky-300/45 ${FOCUS}`}
            >
              {Icon && <Icon size={12} className="text-sky-300" aria-hidden="true" />}
              {t.label}
            </button>
          );
        })}
      </div>
      <button
        type="button"
        aria-label="Hide ready-made teams"
        title="Hide ready-made teams"
        onClick={onHide}
        className={`absolute top-1 right-1 ${phone ? 'w-11 h-11' : 'w-6 h-6'} grid place-items-center rounded-md text-zinc-400 hover:text-zinc-100 hover:bg-white/5 ${FOCUS}`}
      >
        <X size={13} aria-hidden="true" />
      </button>
    </div>
  );
};
