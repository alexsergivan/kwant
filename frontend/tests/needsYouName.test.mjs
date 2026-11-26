// small-label-fixes: the strip is "Needs you" on screen, so no user-facing or aria text in src says the old name.
import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';

const SRC = path.join(path.dirname(new URL(import.meta.url).pathname), '..', 'src');
const files = (d) => fs.readdirSync(d, { withFileTypes: true }).flatMap((e) => (e.isDirectory() ? files(path.join(d, e.name)) : /\.(tsx?)$/.test(e.name) ? [path.join(d, e.name)] : []));

test('no aria-label, title or JSX text says "Needs attention" (comments and old What\'s new notes are fine)', () => {
  const hits = [];
  for (const f of files(SRC)) {
    fs.readFileSync(f, 'utf8').split('\n').forEach((line, i) => {
      const t = line.trim();
      if (t.startsWith('//') || t.startsWith('*') || t.startsWith('/*')) return;
      if (/Needs attention/.test(line)) hits.push(`${path.relative(SRC, f)}:${i + 1}`);
    });
  }
  assert.deepEqual(hits, []);
});
