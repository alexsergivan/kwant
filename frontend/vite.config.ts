import { defineConfig, Plugin } from 'vite';
import react from '@vitejs/plugin-react';
import { createHash } from 'node:crypto';
import { readFileSync, readdirSync } from 'node:fs';
import { join, relative } from 'node:path';

const files = (dir: string): string[] =>
  readdirSync(dir, { withFileTypes: true }).flatMap((e) => (e.isDirectory() ? files(join(dir, e.name)) : [join(dir, e.name)]));

/** Emits sw.js with this build's app shell (every built file plus public/) and a version that
 * changes whenever any of them does, so phones pick up a new build on the next load. */
function pwaServiceWorker(): Plugin {
  let publicDir = '';
  return {
    name: 'kwnt-pwa-service-worker',
    apply: 'build',
    // After Vite's own HTML plugin, so index.html is in the bundle too
    enforce: 'post',
    configResolved(config) {
      publicDir = config.publicDir;
    },
    generateBundle(_options, bundle) {
      const hash = createHash('sha256');
      const shell = new Set<string>();
      for (const [name, chunk] of Object.entries(bundle)) {
        if (name.endsWith('.map')) continue;
        shell.add(`/${name}`);
        hash.update(name).update(chunk.type === 'chunk' ? chunk.code : chunk.source);
      }
      for (const file of files(publicDir)) {
        shell.add(`/${relative(publicDir, file).split('\\').join('/')}`);
        hash.update(file).update(readFileSync(file));
      }
      const template = readFileSync(new URL('./sw.js', import.meta.url), 'utf8');
      hash.update(template);
      this.emitFile({
        type: 'asset',
        fileName: 'sw.js',
        source: template
          .replace('__KWNT_SW_VERSION__', hash.digest('hex').slice(0, 12))
          .replace('__KWNT_SW_SHELL__', JSON.stringify([...shell].sort())),
      });
    },
  };
}

export default defineConfig({
  plugins: [react(), pwaServiceWorker()],
  server: {
    port: 5199,
    proxy: {
      '/api': {
        target: 'http://127.0.0.1:8799',
        changeOrigin: false,
        ws: true,
      },
    },
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
  },
});
