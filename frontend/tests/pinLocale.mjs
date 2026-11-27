// Guest pages format dates, times and numbers in the phone's own locale (`toLocale*(undefined, ...)`).
// The guest tests assert en-US strings, so importing this file pins that default to en-US for the whole
// test process, whatever the machine's LC_ALL/LANG says (an en-GB or de-DE Mac runs the same as CI).
// An explicit locale passed by the code under test is left alone. `withLocale` swaps the default for one call.
let current = 'en-US';
const D = Date.prototype, N = Number.prototype;
const orig = { date: D.toLocaleDateString, time: D.toLocaleTimeString, dt: D.toLocaleString, num: N.toLocaleString };
// `undefined` and `[]` both mean "the runtime's default locale".
const pick = (loc) => (loc === undefined || (Array.isArray(loc) && loc.length === 0) ? current : loc);
D.toLocaleDateString = function (loc, o) { return orig.date.call(this, pick(loc), o); };
D.toLocaleTimeString = function (loc, o) { return orig.time.call(this, pick(loc), o); };
D.toLocaleString = function (loc, o) { return orig.dt.call(this, pick(loc), o); };
N.toLocaleString = function (loc, o) { return orig.num.call(this, pick(loc), o); };
for (const k of ['DateTimeFormat', 'NumberFormat']) {
  const C = Intl[k];
  Intl[k] = new Proxy(C, {
    construct: (t, [loc, o]) => new t(pick(loc), o),
    apply: (t, _, [loc, o]) => t(pick(loc), o),
  });
}

/** Runs `fn` with `loc` as the phone's locale, then puts the default back. */
export function withLocale(loc, fn) {
  const was = current;
  current = loc;
  try { return fn(); } finally { current = was; }
}
