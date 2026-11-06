<script setup lang="ts">
import { computed } from 'vue';

/// Right-hand status column for one facet (Role, Pool or a mood scale).
/// It says where the shown value comes from and offers the one action that
/// makes sense in that state:
/// - suggested: the analyzer's value is shown; ✓ Confirm keeps it as the
///   confirmed value.
/// - confirmed: a confirmed value is shown; ↺ Use suggestion clears it so the
///   analyzer's shows. When the analyzer suggests something else, it is named.
/// - none: nothing to show.
const props = defineProps<{
  /** Label of the analyzer's value, or null when it has none. */
  autoLabel: string | null;
  /** Label of the confirmed value, or null when none was chosen. */
  userLabel: string | null;
  /** Hover text for the suggested state: the analyzer's score. */
  autoHint?: string | null;
  disabled?: boolean;
}>();

const emit = defineEmits<{
  confirm: [];
  useSuggestion: [];
}>();

type State = 'suggested' | 'confirmed' | 'none';

const state = computed<State>(() => {
  if (props.userLabel !== null) return 'confirmed';
  if (props.autoLabel !== null) return 'suggested';
  return 'none';
});

// The suggestion is only worth naming when it differs from the confirmed value.
const differs = computed(
  () => state.value === 'confirmed' && props.autoLabel !== null && props.autoLabel !== props.userLabel,
);
</script>

<template>
  <div class="facet-status d-flex align-center ga-1 text-caption">
    <template v-if="state === 'suggested'">
      <span class="d-inline-flex align-center ga-1 text-medium-emphasis status-text">
        <VIcon size="x-small">mdi-creation</VIcon>Suggested
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
        <VTooltip activator="parent" location="top" open-delay="300">Keep the suggestion</VTooltip>
      </VBtn>
    </template>
    <template v-else-if="state === 'confirmed'">
      <span class="status-text">
        Confirmed<span v-if="differs" class="text-medium-emphasis"> · suggested: {{ autoLabel }}</span>
        <VTooltip v-if="differs && autoHint" activator="parent" location="top" open-delay="300">{{ autoHint }}</VTooltip>
      </span>
      <VBtn
        size="x-small"
        variant="text"
        prepend-icon="mdi-restore"
        :disabled="disabled"
        @click="emit('useSuggestion')"
      >
        Use suggestion
        <VTooltip activator="parent" location="top" open-delay="300">
          Clear the confirmed value; the suggestion shows
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
/* Wide enough for most states ("Confirmed · suggested: Non-combat" + Use
   suggestion), so the column does not jump when the state changes. */
.facet-status {
  min-width: 300px;
  min-height: 28px;
}
.status-text {
  white-space: nowrap;
}
</style>
