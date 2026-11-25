// 36e: Settings › All spaces › "Find a setting". Rows are marked `data-setting-row`, the blocks
// that hold them `data-settings-section`. Words typed hide (data-find-hidden) every row whose
// label and hint don't hold all of them, and every block left with none; empty words show all again.
// The page wrappers' own `hidden` belongs to React and is never touched here.

const SKIP = 'option, datalist, select, script, style, [data-find-skip]';

/** A row's words: its labels and hints, not the choices inside its pickers (the zone list has hundreds). */
export function rowText(el: Element): string {
  let out = '';
  const walk = (n: Node) => {
    if (n.nodeType === 3) out += ' ' + n.nodeValue;
    else if (n.nodeType === 1 && !(n as Element).matches(SKIP)) n.childNodes.forEach(walk);
  };
  walk(el);
  return norm(out);
}

const norm = (s: string) => s.toLowerCase().replace(/[’‘]/g, "'").replace(/\s+/g, ' ').trim();

/** Whether a row's text holds every typed word. */
export function rowMatches(text: string, words: string): boolean {
  const want = norm(words).split(' ').filter(Boolean);
  return want.every((w) => text.includes(w));
}

/** Applies the words under `root`; returns how many rows show (0 with no rows marked). */
export function filterSettingRows(root: Element, words: string): number {
  const on = norm(words) !== '';
  let shown = 0;
  root.querySelectorAll('[data-setting-row]').forEach((row) => {
    const hit = !on || rowMatches(rowText(row), words);
    row.toggleAttribute('data-find-hidden', !hit);
    if (hit) shown++;
  });
  root.querySelectorAll('[data-settings-section]').forEach((sec) => {
    const any = !on || !!sec.querySelector('[data-setting-row]:not([data-find-hidden])');
    sec.toggleAttribute('data-find-hidden', !any);
    // A match inside the closed "For developers" opens it, so the row can be seen.
    if (on && any && sec instanceof HTMLDetailsElement) sec.open = true;
  });
  return shown;
}
