// The pocket guest page: a separate, plain-TypeScript bundle (no React), served by pa1's guest port.
// GET /p/<code> -> dist/pocket/index.html; GET /p/_a/<file> -> dist/pocket/<file> (hashed, immutable).
// Runs after the app build (which empties dist/), and empties only dist/pocket.
import { defineConfig } from 'vite';
import { fileURLToPath } from 'node:url';

const here = fileURLToPath(new URL('.', import.meta.url));

export default defineConfig({
  root: here,
  base: '/p/_a/',
  publicDir: false,
  logLevel: 'warn',
  build: {
    outDir: fileURLToPath(new URL('../dist/pocket', import.meta.url)),
    emptyOutDir: true,
    assetsDir: '',
    // Fonts stay files: a data: font would need font-src data:, which the pocket CSP does not allow.
    assetsInlineLimit: 0,
    cssCodeSplit: false,
    modulePreload: { polyfill: false },
    target: 'es2020',
    reportCompressedSize: false,
    rollupOptions: {
      output: {
        entryFileNames: 'g-[hash].js',
        chunkFileNames: 'g-[hash].js',
        assetFileNames: (a) => (a.names?.[0] ?? a.name ?? '').endsWith('.woff2') ? 'inter-guest-[hash][extname]' : 'g-[hash][extname]',
      },
    },
  },
});
