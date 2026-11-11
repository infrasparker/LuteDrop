import { defineStore } from 'pinia';
import { ref } from 'vue';

// Where the user is on the Soundstage: the open board tab, the selected scene
// in each board, and which tracks are expanded. Kept in a store rather than
// in the components so it survives leaving the page (e.g. Find similar opens
// Scry, and Back returns to the same spot).
export const useSoundstageUiStore = defineStore('soundstageUi', () => {
  const editTrackId = ref<string | null>(null);
  const boardId = ref<string | null>(null);
  const sceneByBoard = ref<Record<string, string | null>>({});
  const expandedTrackIds = ref<Set<string>>(new Set());
  // A track to scroll into view the next time the Soundstage shows it.
  const focusTrackId = ref<string | null>(null);

  function selectScene(board: string, sceneId: string | null): void {
    const next = { ...sceneByBoard.value };
    next[board] = sceneId;
    sceneByBoard.value = next;
  }

  function setTrackExpanded(trackId: string, expanded: boolean): void {
    if (expanded) expandedTrackIds.value.add(trackId);
    else expandedTrackIds.value.delete(trackId);
  }

  return { editTrackId, boardId, sceneByBoard, expandedTrackIds, focusTrackId, selectScene, setTrackExpanded };
});
