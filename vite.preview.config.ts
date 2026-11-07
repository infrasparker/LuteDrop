// UI preview harness: the real app in a plain browser, with Tauri's IPC
// mocked from fixtures (see ui-preview/README.md). `npm run preview:ui`.
// Separate from vite.config.ts, so `npm run build` and the Tauri build never
// see it.
import { defineConfig } from 'vite';
import vue from '@vitejs/plugin-vue';
import vuetify from 'vite-plugin-vuetify';
import { resolve } from 'path';

export default defineConfig({
  root: resolve(__dirname, 'ui-preview'),
  plugins: [vue(), vuetify({ autoImport: true })],
  resolve: {
    alias: {
      '@generated': resolve(__dirname, 'generated'),
    },
  },
  clearScreen: false,
  server: {
    port: 1440,
    strictPort: true,
    // The app's source lives outside the preview root.
    fs: { allow: [resolve(__dirname)] },
  },
});
