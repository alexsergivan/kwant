/** Always tunnel through Kwnt's public origin, including HTTPS reverse proxies. */
export function browserStreamUrl(location: { protocol: string; host: string }, id: string) {
  return (location.protocol === 'https:' ? 'wss://' : 'ws://') + location.host + '/api/sandbox/' + encodeURIComponent(id) + '/stream';
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
