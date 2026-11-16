// Dev-only build of the streaming harness (perf/stream/README.md). Run from frontend/:
//   npx vite build --config perf/stream/vite.config.mts
// Same React plugin and Tailwind setup as the app; "What's new" is stubbed (the harness never opens it).
import { defineConfig, Plugin } from 'vite';
import react from '@vitejs/plugin-react';
import { fileURLToPath } from 'node:url';

const here = fileURLToPath(new URL('.', import.meta.url));
const whatsNewStub = (): Plugin => ({
  name: 'stream-harness-whats-new',
  resolveId: (id) => (id === 'virtual:kwant-whatsnew' ? '\0whatsnew-stub' : null),
  load: (id) => (id === '\0whatsnew-stub' ? 'export const entries = [];\nexport const changelog = {};\n' : null),
});

export default defineConfig({
  root: here,
  base: './',
  publicDir: false,
  plugins: [react(), whatsNewStub()],
  build: { outDir: 'dist', emptyOutDir: true, target: 'es2022' },
});
