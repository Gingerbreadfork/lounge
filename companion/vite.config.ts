import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// Served by the launcher's own HTTP server on the TV; in development it proxies to a TV.
export default defineConfig({
  plugins: [svelte()],
  build: { outDir: 'dist', emptyOutDir: true, target: 'es2022', sourcemap: false },
  server: {
    proxy: {
      '/api': { target: (globalThis as any).process?.env?.LOUNGE_TV ?? 'http://tv.local:8787', ws: true, changeOrigin: true },
    },
  },
});
