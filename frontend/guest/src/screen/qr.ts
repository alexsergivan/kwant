// The lobby's join QR: kwant's own encoder (frontend/src/deviceSignIn.ts qrPath, the same `uqr` package the app
// already ships), drawn as one SVG path. Dark modules only; the white comes from CSS.
import { encode } from 'uqr';

/** The QR code for `text` as one SVG path inside a `size` x `size` box, quiet zone included. */
export function qrPath(text: string): { size: number; d: string } {
  const qr = encode(text, { ecc: 'M', border: 4 });
  let d = '';
  qr.data.forEach((row, y) =>
    row.forEach((dark, x) => {
      if (dark) d += `M${x} ${y}h1v1h-1z`;
    }),
  );
  return { size: qr.size, d };
}
