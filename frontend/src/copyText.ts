/** Puts text on the clipboard. True only when the browser says it did; false when it refused (an insecure
 *  origin, a denied permission) or has no clipboard at all. Callers say "Copied" or "Couldn't copy" from this. */
export async function copyText(text: string): Promise<boolean> {
  const clip = typeof navigator !== 'undefined' ? navigator.clipboard : undefined;
  if (!clip || typeof clip.writeText !== 'function') return false;
  try {
    await clip.writeText(text);
    return true;
  } catch {
    return false;
  }
}
