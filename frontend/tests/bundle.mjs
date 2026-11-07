// Bundles a TypeScript module (with its imports) for the node test runner.
import { build } from 'esbuild';
import { readFileSync, readdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));

/** The "What's new" content as vite.config.ts builds it: the real files unless a test passes its own. */
export function whatsNewModule(content) {
  if (content) return `export const entries = ${JSON.stringify(content.entries)};\nexport const changelog = ${JSON.stringify(content.changelog ?? {})};\n`;
  const dir = new URL('../src/whatsnew/', import.meta.url);
  const entries = readdirSync(dir)
    .filter((n) => /^\d+\.\d+\.\d+\.json$/.test(n))
    .map((n) => JSON.parse(readFileSync(new URL(n, dir), 'utf8')));
  const log = readFileSync(new URL('../../CHANGELOG.md', import.meta.url), 'utf8');
  const section = (v) => {
    const lines = log.split('\n');
    const head = lines.findIndex((l) => l.trimEnd().startsWith(`## [Mac ${v}]`));
    if (head < 0) return null;
    let end = lines.findIndex((l, i) => i > head && l.startsWith('## '));
    if (end < 0) end = lines.length;
    return lines.slice(head + 1, end).join('\n').trim() || null;
  };
  return whatsNewModule({ entries, changelog: Object.fromEntries(entries.map((e) => [e.version, section(e.version)])) });
}

const virtual = (whatsNew) => ({
  name: 'kwnt-virtual',
  setup(b) {
    b.onResolve({ filter: /^virtual:kwant-whatsnew$/ }, (a) => ({ path: a.path, namespace: 'kwnt-virtual' }));
    b.onLoad({ filter: /.*/, namespace: 'kwnt-virtual' }, () => ({ contents: whatsNewModule(whatsNew), loader: 'js' }));
  },
});

/** `opts.whatsNew`: { entries, changelog } to bundle instead of src/whatsnew and CHANGELOG.md. */
export async function loadBundled(contents, opts = {}) {
  const out = await build({
    stdin: { contents, resolveDir: root, loader: 'tsx' },
    bundle: true,
    write: false,
    format: 'esm',
    platform: 'node',
    jsx: 'automatic',
    logLevel: 'silent',
    define: { 'process.env.NODE_ENV': '"production"' },
    plugins: [virtual(opts.whatsNew)],
  });
  return import(`data:text/javascript;base64,${Buffer.from(out.outputFiles[0].text).toString('base64')}`);
}
