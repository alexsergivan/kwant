import { useEffect, useRef, useState } from 'react';
import type { BrowserFrame } from './useBrowserStream';

/**
 * Chrome's screencast sends a blank white picture while a tab commits a new page, before the page
 * paints its own background (reproduced with the live view's own calls: frame 1 of a navigate to a
 * dark page is pure white). On the dark pane that read as a light grey flash. The live view keeps
 * the last good picture while such a frame is in, for at most HOLD_LIGHT_MS, so a page that really
 * is white still shows.
 */
export const HOLD_LIGHT_MS = 1500;

/** Only pictures this small (base64 chars) are checked: a blank 1280x800 frame is about 3.5k, a real page several times more. */
export const PROBE_MAX_CHARS = 24_000;

/** A picture that is almost one flat light colour (RGBA pixels of a small downscale). */
export function isBlankLight(px: ArrayLike<number>): boolean {
  let sum = 0, min = 255, max = 0, n = 0;
  for (let i = 0; i + 2 < px.length; i += 4) {
    const y = 0.2126 * px[i] + 0.7152 * px[i + 1] + 0.0722 * px[i + 2];
    sum += y; n++;
    if (y < min) min = y;
    if (y > max) max = y;
  }
  return n > 0 && sum / n >= 180 && max - min <= 16;
}

/** Whether to show an incoming frame now or keep the last good one: a blank light frame is held, but never past HOLD_LIGHT_MS. */
export function steadyDecision(light: boolean, heldSince: number | null, now: number): 'show' | 'hold' {
  if (!light) return 'show';
  return heldSince !== null && now - heldSince >= HOLD_LIGHT_MS ? 'show' : 'hold';
}

export type Probe = (image: string) => Promise<boolean>;

const canDecode = () => typeof createImageBitmap === 'function' && typeof OffscreenCanvas === 'function';

/** Decodes a frame small and says whether it is a blank light one. Anything that fails counts as a real picture. */
export const probeBlankLight: Probe = async (image) => {
  try {
    const b64 = image.startsWith('data:') ? image.slice(image.indexOf(',') + 1) : image;
    const bin = atob(b64);
    const bytes = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
    const bitmap = await createImageBitmap(new Blob([bytes], { type: 'image/jpeg' }), { resizeWidth: 32, resizeHeight: 20 });
    const canvas = new OffscreenCanvas(32, 20);
    const ctx = canvas.getContext('2d');
    if (!ctx) return false;
    ctx.drawImage(bitmap, 0, 0);
    bitmap.close();
    return isBlankLight(ctx.getImageData(0, 0, 32, 20).data);
  } catch {
    return false;
  }
};

/**
 * The frame the live view shows: the stream's newest, except that a blank light one waits behind the
 * last good picture (or the dark pane) for up to HOLD_LIGHT_MS. A held frame is acknowledged here,
 * since no <img> load will do it. `holding`: a frame is in but nothing good is shown yet.
 */
export function useSteadyFrame(
  frame: BrowserFrame | null,
  acknowledge: (id: number) => void,
  probe: Probe | null = canDecode() ? probeBlankLight : null,
): { frame: BrowserFrame | null; holding: boolean } {
  const [shown, setShown] = useState<BrowserFrame | null>(null);
  const heldSince = useRef<number | null>(null);
  const onScreen = useRef<BrowserFrame | null>(null);
  const shownLight = useRef(false);
  // A frame with the picture already on screen leaves <img src> unchanged, so no load acknowledges it
  const put = (f: BrowserFrame | null) => {
    if (f && onScreen.current && f.id !== onScreen.current.id && f.image === onScreen.current.image) acknowledge(f.id);
    onScreen.current = f;
    setShown(f);
  };
  const passThrough = !frame || !probe || frame.image.length > PROBE_MAX_CHARS;

  useEffect(() => {
    if (passThrough) {
      heldSince.current = null;
      shownLight.current = false;
      put(frame);
      return;
    }
    const f = frame as BrowserFrame;
    let live = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const show = (light = false) => {
      heldSince.current = null;
      shownLight.current = light;
      put(f);
    };
    (probe as Probe)(f.image).then(
      (light) => {
        if (!live) return;
        const now = Date.now();
        if (light && heldSince.current === null) heldSince.current = now;
        if (light && shownLight.current) return show(true);
        if (steadyDecision(light, heldSince.current, now) === 'show') return show(light);
        acknowledge(f.id);
        timer = setTimeout(() => { if (live) show(true); }, HOLD_LIGHT_MS - (now - (heldSince.current ?? now)));
      },
      () => { if (live) show(); },
    );
    return () => {
      live = false;
      if (timer) clearTimeout(timer);
    };
  // eslint-disable-next-line react-hooks/exhaustive-deps -- put only reads refs and acknowledge
  }, [frame, passThrough, probe, acknowledge]);

  if (passThrough) return { frame, holding: false };
  // While a frame is checked or held, the last good picture of this stream stays up (or the dark pane)
  return { frame: shown, holding: !shown };
}

/** What the browser pane shows: a picture, the dark pane while one is on its way, or the idle card. */
export type PaneFrame = { image: string } | { hold: true } | { idle: true };

/**
 * 28e: the picture for the pane. After a switch to another page's tab the new stream has no frame for
 * a moment; the app-level screenshot (not tied to any page, often the blank one dimmed to grey) used
 * to fill that gap. While a chosen page's stream hasn't shown its own good frame, the dark pane holds.
 * `streaming`: the page stream is in use (not a real window, not one on its way). `switching`: the
 * chosen page changed and the stream still belongs to the last one. Without a chosen page,
 * or with no stream, the app-level screenshot is still the fallback, as before.
 */
export function paneFrame(input: {
  blank: boolean;
  frame: BrowserFrame | null;
  holding: boolean;
  streaming: boolean;
  /** The chosen page changed and the stream hasn't reconnected for it yet. */
  switching: boolean;
  streamPage: string | null;
  chosenPage: string | null;
  screenshot: string | null | undefined;
}): PaneFrame {
  const { blank, frame, holding, streaming, switching, streamPage, chosenPage, screenshot } = input;
  // The renders right after the click still have the last page's stream (its address, its frame): hold
  if (streaming && switching) return { hold: true };
  if (blank) return { idle: true };
  if (streaming && chosenPage !== null) {
    // Only this page's own picture: never the app-level screenshot, never the last page's frame
    return frame?.image && streamPage === chosenPage ? { image: frame.image } : { hold: true };
  }
  const image = frame?.image || screenshot;
  if (image) return { image };
  return holding ? { hold: true } : { idle: true };
}
