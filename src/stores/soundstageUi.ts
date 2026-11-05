import { defineStore } from 'pinia';
import { ref } from 'vue';

// Where the user is on the Soundstage: the open group tab, the selected comp
// in each group, and which tracks are expanded. Kept in a store rather than
// in the components so it survives leaving the page (e.g. Find similar opens
// the Vibe view, and Back returns to the same spot).
export const useSoundstageUiStore = defineStore('soundstageUi', () => {
  const groupId = ref<string | null>(null);
  const compByGroup = ref<Record<string, string>>({});
  const expandedTrackIds = ref<Set<string>>(new Set());
  // A track to scroll into view the next time the Soundstage shows it.
  const focusTrackId = ref<string | null>(null);

  function selectComp(group: string, compId: string | null): void {
    const next = { ...compByGroup.value };
    if (compId) next[group] = compId;
    else delete next[group];
    compByGroup.value = next;
  }

  function setTrackExpanded(trackId: string, expanded: boolean): void {
    if (expanded) expandedTrackIds.value.add(trackId);
    else expandedTrackIds.value.delete(trackId);
  }

  return { groupId, compByGroup, expandedTrackIds, focusTrackId, selectComp, setTrackExpanded };
});
