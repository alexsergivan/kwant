import React from 'react';
import { badgeText } from '../away';

export interface PhoneTabItem<T extends string> {
  id: T;
  label: string;
  icon: React.ReactNode;
  badge?: number;
  /** amber = needs you (Inbox); violet = an offer, like the Idea jar's ready takes. */
  badgeTone?: 'amber' | 'violet';
  /** What the count means, for the aria-label: "Inbox, 2 waiting" / "Jar, 2 takes ready". */
  badgeNoun?: string;
}

/** The phone's bottom tab bar (spec A6): 56px plus the home-indicator inset, the app's own violet
 *  palette, a 24×3 bar over the active tab and an 18px amber count on the icon's top-right. */
export function PhoneTabBar<T extends string>({
  label,
  tabs,
  active,
  onSelect,
}: {
  label: string;
  tabs: PhoneTabItem<T>[];
  active: T;
  onSelect: (id: T) => void;
}) {
  return (
    <nav
      aria-label={label}
      data-tab-bar
      className={`shrink-0 grid ${tabs.length >= 5 ? 'grid-cols-5' : 'grid-cols-4'} border-t border-white/[0.07] bg-[#0e0a21] pb-[env(safe-area-inset-bottom)]`}
    >
      {tabs.map((t) => {
        const on = t.id === active;
        return (
          <button
            key={t.id}
            type="button"
            aria-current={on ? 'page' : undefined}
            aria-label={t.badge ? `${t.label}, ${t.badge} ${t.badgeNoun ?? 'waiting'}` : undefined}
            data-tab={t.id}
            onClick={() => onSelect(t.id)}
            className={`relative h-14 flex flex-col items-center justify-center gap-0.5 text-xs leading-4 font-medium focus-visible:outline focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-[#b8adff] ${
              on ? 'text-[#e8e4ff]' : 'text-[#a39dcc] hover:text-[#e8e4ff]'
            }`}
          >
            {on && (
              <span
                aria-hidden="true"
                data-tab-active-bar
                className="absolute top-0 left-1/2 -translate-x-1/2 w-6 h-[3px] rounded-b-sm bg-[#9d8fff]"
              />
            )}
            <span className="relative">
              {t.icon}
              {!!t.badge && (
                <span
                  aria-hidden="true"
                  data-tab-badge
                  data-tone={t.badgeTone ?? 'amber'}
                  className={`absolute top-[-4px] left-[10px] h-[18px] min-w-[18px] px-1 rounded-full ${
                    t.badgeTone === 'violet'
                      ? 'bg-[#8b7bff] text-white text-[11px] leading-[18px] font-semibold text-center ring-2 ring-[#0e0a21]'
                      : 'bg-[#fcd34d] text-[#1b1033] text-[11px] leading-[18px] font-semibold text-center'
                  }`}
                >
                  {badgeText(t.badge)}
                </span>
              )}
            </span>
            {t.label}
          </button>
        );
      })}
    </nav>
  );
}
