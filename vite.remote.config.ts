import { defineConfig } from 'vite';
import vue from '@vitejs/plugin-vue';
import vuetify from 'vite-plugin-vuetify';
import { resolve } from 'path';

export default defineConfig({
  plugins: [
    vue(),
    vuetify({ autoImport: true }),
  ],
  resolve: {
    alias: {
      '@generated': resolve(__dirname, 'generated'),
    },
  },
  root: 'src/remote',
  server: {
    port: 1421,
    strictPort: true,
    host: true,
  },
  build: {
    outDir: '../../dist-remote',
    emptyOutDir: true,
  },
});
