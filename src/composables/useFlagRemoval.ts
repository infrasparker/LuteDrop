import { useAppStore } from '../stores/appStore';
import { useToast } from './useToast';

/// Removes a song's flag on a mood (the Delete key on the Moods page), with
/// an Undo on the toast that sets the old kind and band again.
export function useFlagRemoval() {
  const store = useAppStore();
  const toast = useToast();

  async function removeFlag(entryId: string, mood: string): Promise<void> {
    const old = store.findLibraryEntry(entryId)?.moods.find((m) => m.mood === mood)?.flag;
    if (!old) return;
    const ok = await store.command('clear_mood_flag', { entryId, mood });
    if (!ok) return;
    toast.showInfo('Flag removed', {
      label: 'Undo',
      run: () => void store.command('set_mood_flag', { entryId, mood, kind: old.kind, band: old.kind === 'band' ? old.band : null }),
    });
  }

  return { removeFlag };
}
