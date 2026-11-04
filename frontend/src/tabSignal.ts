// The browser tab says what the page can't while it is in the background: the title carries how many
// things need the person, and the favicon gets a violet dot while any bot works. Pure helpers plus a
// small controller that tests drive with a fake document.

const COUNT_PREFIX = /^\(\d+\)\s*/;

/** "(2) kwant" for 2 things that need you; the plain title for none. Keeps whatever title the page has. */
export function tabTitle(current: string, needsYou: number): string {
  const base = (current || 'kwant').replace(COUNT_PREFIX, '');
  return needsYou > 0 ? `(${needsYou > 99 ? '99+' : needsYou}) ${base}` : base;
}

/** What needs the person: bots waiting on a question or browser step, plus approvals waiting. */
export function needsYouCount(botStatus: Readonly<Record<string, string>>, pendingApprovals: number): number {
  return Object.values(botStatus).filter((s) => s === 'needs_you').length + Math.max(0, pendingApprovals);
}

/** The dot: bottom-right, a quarter of the icon, with a ring in the icon's dark so it reads on any tab. */
export function dotGeometry(size: number): { cx: number; cy: number; r: number; ring: number } {
  const r = Math.round(size * 0.2);
  const ring = Math.max(1, Math.round(size * 0.05));
  return { cx: size - r - ring - 1, cy: size - r - ring - 1, r, ring };
}

interface LinkLike { href: string; type?: string }
export interface TabDoc {
  title: string;
  iconLink(): LinkLike | null;
  /** The icon with the violet dot as an image URL, or null when it can't be drawn. */
  drawDotted(originalHref: string): Promise<string | null>;
}

/** Applies the count to the title and the dot to the icon; each call is idempotent. */
export function createTabSignal(doc: TabDoc) {
  let original: { href: string; type?: string } | null = null;
  let wantDot = false;
  let drawing = 0;
  return {
    setNeedsYou(n: number) {
      const next = tabTitle(doc.title, n);
      if (doc.title !== next) doc.title = next;
    },
    async setWorking(working: boolean) {
      const link = doc.iconLink();
      if (!link || wantDot === working) return;
      wantDot = working;
      if (working) {
        original ??= { href: link.href, type: link.type };
        const ticket = ++drawing;
        const url = await doc.drawDotted(original.href);
        // Idle again (or a newer draw) while the image was loading: leave the icon alone.
        if (url && wantDot && ticket === drawing) {
          link.href = url;
          link.type = 'image/png';
        }
      } else {
        drawing++;
        if (original) {
          link.href = original.href;
          if (original.type !== undefined) link.type = original.type;
        }
      }
    },
  };
}

/** The real document: draws icon.svg (or whatever the page links) onto a canvas with the dot on it. */
export function browserTabDoc(): TabDoc {
  return {
    get title() { return document.title; },
    set title(v: string) { document.title = v; },
    iconLink: () => {
      let el = document.querySelector<HTMLLinkElement>('link[rel~="icon"]');
      if (!el) {
        el = document.createElement('link');
        el.rel = 'icon';
        document.head.appendChild(el);
      }
      return el;
    },
    drawDotted: (href) =>
      new Promise((resolve) => {
        const img = new Image();
        img.onload = () => {
          try {
            const size = 64;
            const c = document.createElement('canvas');
            c.width = c.height = size;
            const g = c.getContext('2d');
            if (!g) return resolve(null);
            g.drawImage(img, 0, 0, size, size);
            const { cx, cy, r, ring } = dotGeometry(size);
            g.fillStyle = '#120e26';
            g.beginPath(); g.arc(cx, cy, r + ring, 0, Math.PI * 2); g.fill();
            g.fillStyle = '#a78bfa';
            g.beginPath(); g.arc(cx, cy, r, 0, Math.PI * 2); g.fill();
            resolve(c.toDataURL('image/png'));
          } catch { resolve(null); }
        };
        img.onerror = () => resolve(null);
        img.src = href;
      }),
  };
}
