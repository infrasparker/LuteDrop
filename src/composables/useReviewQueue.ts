import { computed, ref } from 'vue';
import type { LibraryEntry } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import { compareReview, needsReview } from '../utils/pools';

// Session state, kept at module level so leaving the Review tab and coming
// back keeps it. Nothing here is saved: Skip is ordering only.
/// Ids skipped this session, in the order they were skipped.
const skipped = ref<string[]>([]);
/// How many cards were approved from the Review tab this session.
const approvedCount = ref(0);

/// The Review queue: music with a pool to look at that has not been reviewed,
/// newest first and least sure first (see `compareReview`). Skipped entries
/// move to the end, in the order they were skipped.
export function useReviewQueue() {
  const store = useAppStore();

  const pending = computed<LibraryEntry[]>(() => store.libraryEntries.filter(needsReview).sort(compareReview));

  const queue = computed<LibraryEntry[]>(() => {
    const skippedSet = new Set(skipped.value);
    const fresh = pending.value.filter((e) => !skippedSet.has(e.id));
    const byId = new Map(pending.value.map((e) => [e.id, e]));
    const later = skipped.value.map((id) => byId.get(id)).filter((e): e is LibraryEntry => e !== undefined);
    return [...fresh, ...later];
  });

  /// Number of skipped entries still waiting.
  const skippedWaiting = computed(() => {
    const ids = new Set(pending.value.map((e) => e.id));
    return skipped.value.filter((id) => ids.has(id)).length;
  });

  /// The badge count: every entry still waiting for review.
  const count = computed(() => pending.value.length);

  function skip(entryId: string): void {
    skipped.value = [...skipped.value.filter((id) => id !== entryId), entryId];
    // Once every waiting entry has been skipped, start a fresh pass.
    if (skippedWaiting.value >= pending.value.length) skipped.value = [];
  }

  function noteApproved(): void {
    approvedCount.value++;
  }

  return { queue, count, skippedWaiting, approvedCount, skip, noteApproved };
}
