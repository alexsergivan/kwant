/** Always tunnel through Kwnt's public origin, including HTTPS reverse proxies. */
/** The live view of a team's browser; `page` (0.9.3) picks one of its pages, else the server's default. */
export function browserStreamUrl(location: { protocol: string; host: string }, id: string, page?: string | null) {
  const base = (location.protocol === 'https:' ? 'wss://' : 'ws://') + location.host + '/api/sandbox/' + encodeURIComponent(id) + '/stream';
  // A job page's key holds '#' (team#bot#chat), so it must be encoded or it would read as a fragment
  return page ? `${base}?page=${encodeURIComponent(page)}` : base;
}

/** Map a pointer through the centered letterboxing applied by object-fit: contain. */
export function mapBrowserPoint(
  clientX: number,
  clientY: number,
  bounds: { left: number; top: number; width: number; height: number },
  imageWidth: number,
  imageHeight: number
): { x: number; y: number } | null {
  if (bounds.width <= 0 || bounds.height <= 0 || imageWidth <= 0 || imageHeight <= 0) {
    return null;
  }

  const scale = Math.min(bounds.width / imageWidth, bounds.height / imageHeight);
  const left = bounds.left + (bounds.width - imageWidth * scale) / 2;
  const top = bounds.top + (bounds.height - imageHeight * scale) / 2;
  const x = (clientX - left) / scale;
  const y = (clientY - top) / scale;
  if (x < 0 || y < 0 || x >= imageWidth || y >= imageHeight) return null;

  return { x: Math.floor(x), y: Math.floor(y) };
}
