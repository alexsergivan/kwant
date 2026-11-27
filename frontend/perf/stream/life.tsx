// Living avatars harness (dev only, never shipped; ui_budget "life" case). 30 bots on one page: 28 sidebar rows
// (list zone: blinks, hover glance), a chat header avatar (header zone) and a 96 px Bot home avatar (full zone),
// all on the one shared clock. The runner counts React commits (must stay 0: frames are DOM attributes), the clock's
// timers (at most 1) and the frames' p95 while the avatars live for 20 s.
import React from 'react';
import { createRoot } from 'react-dom/client';
import '../../src/styles/fonts.css';
import '../../src/styles/modern.css';
import { ModernAvatar, AVATAR_STYLES } from '../../src/components/ModernAvatar';
import { lifeStats } from '../../src/avatarLife';

const bots = Array.from({ length: 30 }, (_, i) => ({ id: `k${i}`, name: `Bot ${i}`, sprite: AVATAR_STYLES[i % AVATAR_STYLES.length].id }));
const list = bots.slice(0, 28);
const header = bots[28];
const home = bots[29];

const App = () => (
  <div className="flex h-screen">
    <aside className="w-72 shrink-0 overflow-hidden bg-surface-side p-2 flex flex-col gap-0.5">
      {list.map((b) => (
        <div key={b.id} data-life-row="" className="relative flex items-center gap-2.5 px-2 py-1.5 rounded-xl text-zinc-300">
          <ModernAvatar role={b.sprite} name={b.name} size={28} kwantId={b.id} life="list" modelBadge={false} />
          <span className="text-sm">{b.name}</span>
        </div>
      ))}
    </aside>
    <main className="flex-1 flex flex-col">
      <header className="flex items-center gap-3 px-4 h-14 border-b border-hairline">
        <ModernAvatar role={header.sprite} name={header.name} size={34} kwantId={header.id} className="rounded-xl" life="header" modelBadge={false} />
        <span>{header.name}</span>
      </header>
      <div className="flex-1 grid place-items-center">
        <ModernAvatar role={home.sprite} name={home.name} size={96} kwantId={home.id} className="rounded-2xl shadow-raised" life="full" modelBadge={false} />
      </div>
    </main>
  </div>
);

createRoot(document.getElementById('root')!).render(<App />);
(window as unknown as { __life: unknown }).__life = {
  ready: () => document.querySelectorAll('[data-life-zone]').length === 30,
  stats: lifeStats,
};
