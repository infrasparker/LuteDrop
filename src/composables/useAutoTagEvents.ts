import { ref, readonly } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { AppState, AutoTagProgress } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import { useToast } from './useToast';

// Shared state for views that show a run: the latest progress event, and a
// counter that goes up each time a run finishes so a view can re-read the
// report.
const progress = ref<AutoTagProgress | null>(null);
const completedRuns = ref(0);

// Registered once at module scope, like the library import listeners: a
// finished auto-tag run wrote tags in the backend, so the store reloads.
let listenersRegistered = false;

export function useAutoTagEvents() {
  if (!listenersRegistered) {
    listenersRegistered = true;

    const store = useAppStore();
    const toast = useToast();

    listen<AutoTagProgress>('autotag-progress', (event) => {
      progress.value = event.payload;
    });

    listen('autotag-complete', async () => {
      progress.value = null;
      completedRuns.value++;
      try {
        const state = await invoke<AppState>('load_data');
        store.hydrate(state);
      } catch (e) {
        toast.showError(`Failed to refresh library: ${e}`);
      }
    });
  }

  return { progress: readonly(progress), completedRuns: readonly(completedRuns) };
}
