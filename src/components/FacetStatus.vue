<script setup lang="ts">
import { computed } from 'vue';

/// Right-hand status column for one facet (Role, Pace or a mood scale).
/// It says where the shown value comes from and offers the one action that
/// makes sense in that state:
/// - auto: the analyzer's value is shown; ✓ Confirm makes it yours.
/// - user: your value is shown; ↺ Use auto removes it so the analyzer's shows.
///   When the analyzer has a different value, it is named.
/// - none: nothing to show.
const props = defineProps<{
  /** Label of the analyzer's value, or null when it has none. */
  autoLabel: string | null;
  /** Label of your value, or null when you have not chosen one. */
  userLabel: string | null;
  /** Hover text for the auto state: the analyzer's score. */
  autoHint?: string | null;
  disabled?: boolean;
}>();

const emit = defineEmits<{
  confirm: [];
  useAuto: [];
}>();

type State = 'auto' | 'user' | 'none';

const state = computed<State>(() => {
  if (props.userLabel !== null) return 'user';
  if (props.autoLabel !== null) return 'auto';
  return 'none';
});

// The analyzer's value is only worth naming when it differs from yours.
const differs = computed(
  () => state.value === 'user' && props.autoLabel !== null && props.autoLabel !== props.userLabel,
);
</script>

<template>
  <div class="facet-status d-flex align-center ga-1 text-caption">
    <template v-if="state === 'auto'">
      <span class="d-inline-flex align-center ga-1 text-medium-emphasis status-text">
        <VIcon size="x-small">mdi-creation</VIcon>Auto
        <VTooltip v-if="autoHint" activator="parent" location="top" open-delay="300">{{ autoHint }}</VTooltip>
      </span>
      <VBtn
        size="x-small"
        variant="text"
        prepend-icon="mdi-check"
        :disabled="disabled"
        @click="emit('confirm')"
      >
        Confirm
        <VTooltip activator="parent" location="top" open-delay="300">Make this yours</VTooltip>
      </VBtn>
    </template>
    <template v-else-if="state === 'user'">
      <span class="status-text">
        Yours<span v-if="differs" class="text-medium-emphasis"> · auto: {{ autoLabel }}</span>
      </span>
      <VBtn
        size="x-small"
        variant="text"
        prepend-icon="mdi-restore"
        :disabled="disabled"
        @click="emit('useAuto')"
      >
        Use auto
        <VTooltip activator="parent" location="top" open-delay="300">
          Remove your choice; the analyzer's value shows
        </VTooltip>
      </VBtn>
    </template>
    <span v-else class="status-text text-medium-emphasis">
      —
      <VTooltip activator="parent" location="top" open-delay="300">Not analyzed</VTooltip>
    </span>
  </div>
</template>

<style scoped>
/* Wide enough for the longest state ("Yours · auto: Non-combat" + Use auto),
   so the column does not jump when the state changes. */
.facet-status {
  min-width: 250px;
  min-height: 28px;
}
.status-text {
  white-space: nowrap;
}
</style>
