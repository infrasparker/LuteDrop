import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { AppState } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import { useToast } from './useToast';

// Registered once at module scope, like the library import listeners: a
// finished auto-tag run wrote tags in the backend, so the store reloads.
let listenersRegistered = false;

export function useAutoTagEvents(): void {
  if (listenersRegistered) return;
  listenersRegistered = true;

  const store = useAppStore();
  const toast = useToast();

  listen('autotag-complete', async () => {
    try {
      const state = await invoke<AppState>('load_data');
      store.hydrate(state);
    } catch (e) {
      toast.showError(`Failed to refresh library: ${e}`);
    }
  });
}
