import { invoke } from '@tauri-apps/api/core';
import type { AppState, ClearedFlag } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import { useToast } from './useToast';

/// Removes a song's flag on a mood (the Delete key on the Moods page), with
/// an Undo on the toast that puts the removed row back exactly (bound, bar
/// and calibration it was made at, wording, and when it was set).
export function useFlagRemoval() {
  const store = useAppStore();
  const toast = useToast();

  async function removeFlag(entryId: string, mood: string): Promise<void> {
    let removed: ClearedFlag['removed'];
    try {
      const out = await invoke<ClearedFlag>('clear_mood_flag', { entryId, mood });
      store.hydrate(out.state);
      removed = out.removed;
    } catch (e) {
      toast.showError(String(e));
      return;
    }
    if (!removed) return;
    const row = removed;
    toast.showInfo('Flag removed', {
      label: 'Undo',
      run: async () => {
        try {
          store.hydrate(await invoke<AppState>('restore_mood_flags', { rows: [row] }));
        } catch (e) {
          toast.showError(String(e));
        }
      },
    });
  }

  return { removeFlag };
}
