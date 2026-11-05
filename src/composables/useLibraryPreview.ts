import { computed } from 'vue';
import { useAppStore } from '../stores/appStore';
import { useAudio } from './useAudio';
import { LIBRARY_PLAYBACK_ID } from '../shared/playback';

// Preview playback of library entries through the shared library sound.
export function useLibraryPreview() {
  const store = useAppStore();
  const audio = useAudio();

  const playingId = computed(() =>
    store.playbackStates.get(LIBRARY_PLAYBACK_ID)?.isPlaying ? store.libraryPlayingEntryId : null,
  );

  function stop(): void {
    if (playingId.value) audio.stopLibrarySound();
  }

  function toggle(entryId: string): void {
    if (playingId.value === entryId) {
      audio.stopLibrarySound();
      return;
    }
    const entry = store.findLibraryEntry(entryId);
    if (!entry) return;
    audio.playLibrarySound(
      entry.id,
      entry.filePath,
      entry.trimStart ?? 0,
      entry.volume,
      entry.trimStart, entry.trimEnd,
      entry.loopStart, entry.loopEnd,
    );
  }

  return { playingId, toggle, stop };
}
