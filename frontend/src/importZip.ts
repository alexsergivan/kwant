// Several picked files → one uncompressed (STORED) zip, so the import server's zip path reads them
// all into one preview. No dependency: local headers, CRC-32 and a central directory.

/** The server's zip limits (src/import/mod.rs). */
export const PICK_LIMITS = { zipBytes: 20 * 1024 * 1024, files: 500, fileBytes: 4 * 1024 * 1024 } as const;

/** Archives the server won't open inside a zip (src/import/archive.rs ARCHIVE_EXTENSIONS). */
const ARCHIVES = ['.zip', '.gz', '.tgz', '.tar', '.7z', '.rar', '.xz', '.bz2', '.zst'];
export const isArchiveName = (name: string) => ARCHIVES.some((e) => name.toLowerCase().endsWith(e));

let table: Uint32Array | null = null;
export function crc32(data: Uint8Array): number {
  if (!table) {
    table = new Uint32Array(256);
    for (let n = 0; n < 256; n++) {
      let c = n;
      for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
      table[n] = c >>> 0;
    }
  }
  let c = 0xffffffff;
  for (let i = 0; i < data.length; i++) c = table[(c ^ data[i]) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

/** Base names only (no folders), and a second "a.json" becomes "a (2).json". A SKILL.md keeps its
 *  name (the server needs it) and goes in its own folder instead: "skill-2/SKILL.md". */
export function zipNames(names: string[]): string[] {
  const used = new Set<string>();
  return names.map((raw) => {
    const base = raw.split(/[\\/]/).pop()?.trim() || 'file';
    const skill = base.toLowerCase() === 'skill.md';
    const dot = base.lastIndexOf('.');
    const [stem, ext] = dot > 0 ? [base.slice(0, dot), base.slice(dot)] : [base, ''];
    let name = base;
    for (let n = 2; used.has(name.toLowerCase()); n++) name = skill ? `skill-${n}/${base}` : `${stem} (${n})${ext}`;
    used.add(name.toLowerCase());
    return name;
  });
}

/** Header bytes the zip adds per file and in total, on top of the files themselves. */
export const zipOverhead = (names: string[]) =>
  names.reduce((n, name) => n + 30 + 46 + 2 * new TextEncoder().encode(name).length, 22);

/** One STORED zip of these files (names as given, UTF-8). */
export function storedZip(files: Array<{ name: string; data: Uint8Array }>): Uint8Array {
  const enc = new TextEncoder();
  const parts: Uint8Array[] = [];
  const central: Uint8Array[] = [];
  let offset = 0;
  // 1980-01-01 00:00: the same bytes every time (the date doesn't matter to the server).
  const time = 0;
  const date = (0 << 9) | (1 << 5) | 1;
  for (const f of files) {
    const name = enc.encode(f.name);
    const crc = crc32(f.data);
    const size = f.data.length;
    const local = new DataView(new ArrayBuffer(30));
    local.setUint32(0, 0x04034b50, true);
    local.setUint16(4, 20, true); // version needed
    local.setUint16(6, 0x0800, true); // UTF-8 names
    local.setUint16(8, 0, true); // STORED
    local.setUint16(10, time, true);
    local.setUint16(12, date, true);
    local.setUint32(14, crc, true);
    local.setUint32(18, size, true);
    local.setUint32(22, size, true);
    local.setUint16(26, name.length, true);
    local.setUint16(28, 0, true);
    parts.push(new Uint8Array(local.buffer), name, f.data);
    const cd = new DataView(new ArrayBuffer(46));
    cd.setUint32(0, 0x02014b50, true);
    cd.setUint16(4, 20, true); // version made by
    cd.setUint16(6, 20, true);
    cd.setUint16(8, 0x0800, true);
    cd.setUint16(10, 0, true);
    cd.setUint16(12, time, true);
    cd.setUint16(14, date, true);
    cd.setUint32(16, crc, true);
    cd.setUint32(20, size, true);
    cd.setUint32(24, size, true);
    cd.setUint16(28, name.length, true);
    // extra, comment, disk, internal attrs: 0
    cd.setUint32(38, 0, true); // external attrs
    cd.setUint32(42, offset, true);
    central.push(new Uint8Array(cd.buffer), name);
    offset += 30 + name.length + size;
  }
  const cdSize = central.reduce((n, p) => n + p.length, 0);
  const end = new DataView(new ArrayBuffer(22));
  end.setUint32(0, 0x06054b50, true);
  end.setUint16(8, files.length, true);
  end.setUint16(10, files.length, true);
  end.setUint32(12, cdSize, true);
  end.setUint32(16, offset, true);
  const all = [...parts, ...central, new Uint8Array(end.buffer)];
  const out = new Uint8Array(all.reduce((n, p) => n + p.length, 0));
  let at = 0;
  for (const p of all) {
    out.set(p, at);
    at += p.length;
  }
  return out;
}

/** Why these picked files can't be sent, in plain words; null when they can. One file is checked as before. */
export function pickProblem(files: ReadonlyArray<{ name: string; size: number }>): string | null {
  if (files.length < 2) return null;
  if (files.some((f) => isArchiveName(f.name))) return 'Pick one zip, or several files that aren’t zips.';
  if (files.length > PICK_LIMITS.files) return `That’s ${files.length} files. Pick up to ${PICK_LIMITS.files} at once.`;
  const big = files.find((f) => f.size > PICK_LIMITS.fileBytes);
  if (big) return `${big.name} is over 4 MB, so it can’t be imported.`;
  const total = files.reduce((n, f) => n + f.size, 0) + zipOverhead(zipNames(files.map((f) => f.name)));
  if (total > PICK_LIMITS.zipBytes) return 'These files add up to more than 20 MB. Pick fewer at a time.';
  return null;
}

/** What to upload for a pick: the one file as it is, or several packed into "bots.zip". */
export async function uploadFor(files: ReadonlyArray<File>): Promise<{ body: Blob; filename: string }> {
  if (files.length === 1) return { body: files[0], filename: files[0].name };
  const names = zipNames(files.map((f) => f.name));
  const data = await Promise.all(files.map(async (f) => new Uint8Array(await f.arrayBuffer())));
  const zip = storedZip(names.map((name, i) => ({ name, data: data[i] })));
  return { body: new Blob([zip.buffer as ArrayBuffer], { type: 'application/zip' }), filename: 'bots.zip' };
}
