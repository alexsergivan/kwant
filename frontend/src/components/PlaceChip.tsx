import React, { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { ChevronsUpDown, Info } from 'lucide-react';
import {
  ELSEWHERE_EVENT,
  PERCH_COPY,
  chipAriaLabel,
  elsewhereCount,
  elsewhereText,
  HOST_MAX,
  islandUrl,
  middleCut,
  onceEvery,
  perchSettings,
  placeOf,
  placesUrl,
  shortcutOptions,
  showPlaceChip,
  type PerchMode,
  type PerchSettings,
  type PerchShortcut,
  type PlaceDot,
} from '../placeChip';

/** 14 px pixel glyphs on a 7x7 grid of 2 px cells (design.md 9.1). */
const Glyph: React.FC<{ kind: 'local' | 'server' }> = ({ kind }) => (
  <svg className="kw-place-glyph" viewBox="0 0 14 14" aria-hidden="true" fill="currentColor">
    {kind === 'local' ? (
      <path d="M2 2h10v2H2zM2 4h2v4H2zM10 4h2v4h-2zM2 8h10v2H2zM0 10h14v2H0z" />
    ) : (
      <path d="M2 1h10v2H2zM2 3h2v2H2zM10 3h2v2h-2zM2 5h10v2H2zM2 8h10v2H2zM2 10h2v2H2zM10 10h2v2h-2zM2 12h10v1H2zM6 3h2v2H6zM6 10h2v2H6z" />
    )}
  </svg>
);

/** The waiting-elsewhere count Rust keeps on the This Mac window; 0 everywhere else. */
export function useElsewhere(): number {
  const [n, setN] = useState(() => elsewhereCount());
  useEffect(() => {
    const on = () => setN(elsewhereCount());
    window.addEventListener(ELSEWHERE_EVENT, on);
    on();
    return () => window.removeEventListener(ELSEWHERE_EVENT, on);
  }, []);
  return n;
}

type ChipProps = {
  phone?: boolean;
  /** Only for shots and tests: a status dot and forced visual states. The app passes none. */
  dot?: PlaceDot;
  demo?: 'hover' | 'focus' | 'open';
};

/** The chip and its row. Renders nothing outside the desktop app. */
export const PlaceChipRow: React.FC<ChipProps> = ({ phone = false, dot = null, demo }) => {
  const elsewhere = useElsewhere();
  const ref = useRef<HTMLButtonElement>(null);
  const gate = useMemo(() => onceEvery(1000), []);
  const [open, setOpen] = useState(false);
  const nameRef = useRef<HTMLSpanElement>(null);
  // A host that still overflows the Sidebar after the 28-character cut is cut shorter in the middle, so its end
  // (the part that tells two servers apart) stays visible. The CSS end-ellipsis is only a last resort.
  const [max, setMax] = useState(HOST_MAX);
  const [, remeasure] = useState(0);
  const shown = showPlaceChip();
  const place = shown ? placeOf() : null;
  const isServer = place?.kind === 'server';
  useLayoutEffect(() => {
    const n = nameRef.current;
    if (isServer && n && n.scrollWidth > n.clientWidth + 0.5) setMax((m) => (m > 12 ? m - 1 : m));
  });
  useEffect(() => {
    if (!isServer) return;
    // Widths change once the web font arrives and when the window is resized: start over from 28 then.
    const again = () => {
      setMax(HOST_MAX);
      remeasure((t) => t + 1);
    };
    document.fonts?.ready.then(again).catch(() => {});
    window.addEventListener('resize', again);
    return () => window.removeEventListener('resize', again);
  }, [isServer]);
  if (!shown || !place) return null;
  const name = place.kind === 'server' ? middleCut(place.full, max) : place.name;
  const click = () => {
    if (!gate()) return;
    const r = ref.current?.getBoundingClientRect();
    setOpen(true);
    window.setTimeout(() => setOpen(false), 1000);
    // Rust's on_navigation refuses this navigation and opens the bundled picker under the chip.
    window.location.assign(placesUrl(r ? r.left : 0, r ? r.bottom : 0));
  };
  return (
    <div className="kw-place kw-place-row" data-phone={phone ? '' : undefined} data-place-row>
      <button
        ref={ref}
        type="button"
        onClick={click}
        className="kw-place-chip"
        aria-label={chipAriaLabel(place.full, elsewhere)}
        aria-haspopup="dialog"
        aria-expanded={open || demo === 'open'}
        title={place.full !== name ? place.full : undefined}
        data-place-chip={place.kind}
        data-hover={demo === 'hover' ? '' : undefined}
        data-focus={demo === 'focus' ? '' : undefined}
      >
        <Glyph kind={place.kind} />
        <span ref={nameRef} className="kw-place-name">{name}</span>
        {elsewhere > 0 ? (
          <span className="kw-place-count" aria-hidden="true" data-place-elsewhere>
            {elsewhereText(elsewhere)}
          </span>
        ) : dot ? (
          <span className="kw-place-dot" data-dot={dot} aria-hidden="true" />
        ) : null}
        <ChevronsUpDown size={13} className="kw-place-chev" aria-hidden="true" />
      </button>
    </div>
  );
};

/** The 3-row perch mark: a little bar with a stepped lip. */
const PerchMark = () => (
  <svg width="18" height="14" viewBox="0 0 18 14" aria-hidden="true" fill="currentColor">
    <path d="M0 2h18v3H0zM2 5h14v2H2zM5 7h8v2H5z" />
  </svg>
);

/** Settings → App settings → Perch. Only in the This Mac window, where Rust sets `__KWANT_ISLAND__`. */
export const PerchSettingsCard: React.FC<{ initial?: PerchSettings | null }> = ({ initial }) => {
  const start = initial === undefined ? perchSettings() : initial;
  const [s, setS] = useState<PerchSettings | null>(start);
  const gate = useMemo(() => onceEvery(1000), []);
  const options = useMemo(() => shortcutOptions(), []);
  if (!s) return null;
  const off = !s.available;
  const write = (mode: PerchMode, shortcut: PerchShortcut) => {
    const url = islandUrl(mode, shortcut);
    if (!url || off || !gate()) return;
    setS({ ...s, mode, shortcut });
    window.location.assign(url);
  };
  const modeLine = PERCH_COPY.modes.find((m) => m.value === s.mode)?.line;
  return (
    <section className="kw-perch" aria-labelledby="perch-title" data-perch-settings data-unavailable={off ? '' : undefined}>
      <div className="kw-perch-title">
        <PerchMark />
        <div>
          <h3 id="perch-title" className="kw-perch-name">{PERCH_COPY.title}</h3>
          <p className="kw-perch-about">{PERCH_COPY.about}</p>
        </div>
      </div>
      <div role="radiogroup" aria-labelledby="perch-title" aria-describedby="perch-mode-line" className="kw-perch-seg">
        {PERCH_COPY.modes.map((m) => (
          <button
            key={m.value}
            type="button"
            role="radio"
            aria-checked={s.mode === m.value}
            disabled={off}
            onClick={() => s.mode !== m.value && write(m.value, s.shortcut)}
            data-perch-mode={m.value}
          >
            {m.label}
          </button>
        ))}
      </div>
      <p id="perch-mode-line" className="kw-perch-line">{modeLine}</p>
      <div className="kw-perch-short" data-off={off || s.mode === 'off' ? '' : undefined}>
        <label htmlFor="perch-shortcut">{PERCH_COPY.shortcutLabel}</label>
        <select
          id="perch-shortcut"
          value={s.shortcut}
          disabled={off || s.mode === 'off'}
          onChange={(e) => write(s.mode, e.target.value as PerchShortcut)}
          data-perch-shortcut
        >
          {options.map((o) => (
            <option key={o.value} value={o.value}>
              {o.label}
            </option>
          ))}
        </select>
      </div>
      {off && (
        <p className="kw-perch-note" data-perch-wayland>
          <Info size={14} aria-hidden="true" />
          <span>{PERCH_COPY.wayland}</span>
        </p>
      )}
    </section>
  );
};
