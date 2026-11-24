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
    // The big screen's CSS (pa5b) and the host remote's (pa4b) are their own files beside their lazy chunks, so phones never fetch them.
    cssCodeSplit: true,
    modulePreload: { polyfill: false },
    target: 'es2020',
    reportCompressedSize: false,
    rollupOptions: {
      output: {
        entryFileNames: 'g-[hash].js',
        // The Wall's big screen (pa14) shares the quiz screen's CSS and QR maker: both go in one screen-kit chunk, so they
        // stay out of the phone budget whichever screen loads first.
        manualChunks: (id) => (/[\\/]src[\\/]screen[\\/](qr\.ts|screen\.css)$|[\\/]node_modules[\\/]uqr[\\/]/.test(id) ? 'screen-kit' : undefined),
        chunkFileNames: (c) => (c.name.startsWith('screen') ? 'g-screen-[hash].js' : c.name === 'remote' ? 'g-remote-[hash].js' : 'g-[hash].js'),
        assetFileNames: (a) => {
          const n = a.names?.[0] ?? a.name ?? '';
          return n.endsWith('.woff2') ? 'inter-guest-[hash][extname]' : n.startsWith('screen') ? 'g-screen-[hash][extname]' : n.startsWith('remote') ? 'g-remote-[hash][extname]' : 'g-[hash][extname]';
        },
      },
    },
  },
});
