import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'vite';
import { createPinia, setActivePinia } from 'pinia';
import { effectScope, nextTick } from 'vue';
import { mockIPC, clearMocks } from '@tauri-apps/api/mocks';
import { useLocalVolume } from '../src/shared/useLocalVolume.ts';

// Load the actual store/action through Vite's TypeScript resolver. No browser,
// DOM, audio engine, or live database is involved.
test('clip volume stays synchronized across editor lifetimes', async (t) => {
  const previousWindow = globalThis.window;
  globalThis.window = { crypto: globalThis.crypto };
  const vite = await createServer({ configFile: false, envFile: false, logLevel: 'silent',
    server: { middlewareMode: true, watch: null }, appType: 'custom' });
  try {
    const { useAppStore } = await vite.ssrLoadModule('/src/stores/appStore.ts');
    const { useAudio } = await vite.ssrLoadModule('/src/composables/useAudio.ts');
    function setup() {
      setActivePinia(createPinia());
      const store = useAppStore();
      store.tracks = [{ id: 'weather', name: 'Weather', clips: [{ id: 'rain', volume: 1 }, { id: 'wind', volume: .7 }] }];
      const volume = () => store.findTrack('weather').clips[0].volume;
      return { store, volume };
    }

    await t.test('a saved clip level survives collapse/reopen and updates another mounted editor', async () => {
      const { store, volume } = setup();
      const calls = [];
      mockIPC(async (command, args) => { calls.push([command, args]); }, { shouldMockEvents: true });
      const audio = useAudio();
      const firstScope = effectScope(); const otherScope = effectScope();
      let saved;
      const first = firstScope.run(() => useLocalVolume(volume, (v) => { saved = audio.setClipVolume('rain', v); }));
      const other = otherScope.run(() => useLocalVolume(volume));
      first.set(.18); await saved; await nextTick();
      firstScope.stop();
      const reopenedScope = effectScope();
      const reopened = reopenedScope.run(() => useLocalVolume(volume));
      assert.equal(reopened.local.value, .18, 'reopened slider read the old full-volume value');
      assert.equal(other.local.value, .18, 'another view of the shared clip stayed stale');
      assert.equal(store.findTrack('weather').clips[1].volume, .7);
      assert.deepEqual(calls, [['set_clip_volume', { clipId: 'rain', volume: .18 }]]);
      reopenedScope.stop(); otherScope.stop();
    });

    await t.test('closing before IPC completes still updates the reopened slider after the save', async () => {
      const { volume } = setup();
      let finish;
      mockIPC(() => new Promise((resolve) => { finish = resolve; }), { shouldMockEvents: true });
      const audio = useAudio();
      const firstScope = effectScope(); let saved;
      const first = firstScope.run(() => useLocalVolume(volume, (v) => { saved = audio.setClipVolume('rain', v); }));
      first.set(.27); firstScope.stop();
      const reopenedScope = effectScope();
      const reopened = reopenedScope.run(() => useLocalVolume(volume));
      finish(); await saved; await nextTick();
      assert.equal(reopened.local.value, .27);
      reopenedScope.stop();
    });

    await t.test('a failed save does not replace the last confirmed clip volume', async () => {
      const { volume } = setup();
      mockIPC(() => { throw new Error('Save failed'); }, { shouldMockEvents: true });
      await useAudio().setClipVolume('rain', .2);
      assert.equal(volume(), 1);
    });
  } finally {
    clearMocks(); globalThis.window = previousWindow; await vite.close();
  }
});
