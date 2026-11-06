<script setup lang="ts">
import { computed } from 'vue';

/// Right-hand status column for the Role facet. A role is always shown (the
/// user's, else the analyzer's, else Non-combat by default), so there are two
/// states:
/// - suggested: the analyzer's value or the default is shown; ✓ Confirm
///   makes it the user's.
/// - set: the user set the value ("Set by you"); ↺ goes back to the
///   suggestion. When the analyzer suggests something else, it is named.
const props = defineProps<{
  /** Label of the analyzer's value, or null when it has none. */
  autoLabel: string | null;
  /** Label of the value the user set, or null when none was set. */
  userLabel: string | null;
  /** Hover text for the suggestion: the analyzer's score, or why the default applies. */
  autoHint?: string | null;
  disabled?: boolean;
}>();

const emit = defineEmits<{
  confirm: [];
  useSuggestion: [];
}>();

const isSet = computed(() => props.userLabel !== null);

// The suggestion is only worth naming when it differs from the value set.
const differs = computed(() => isSet.value && props.autoLabel !== null && props.autoLabel !== props.userLabel);
</script>

<template>
  <div class="facet-status d-flex align-center ga-1 text-caption">
    <template v-if="!isSet">
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
        <VTooltip activator="parent" location="top" open-delay="300">Keep the suggestion as yours</VTooltip>
      </VBtn>
    </template>
    <template v-else>
      <span class="status-text">
        Set by you<span v-if="differs" class="text-medium-emphasis"> · suggested: {{ autoLabel }}</span>
        <VTooltip v-if="differs && autoHint" activator="parent" location="top" open-delay="300">{{ autoHint }}</VTooltip>
      </span>
      <VBtn
        icon="mdi-restore"
        size="x-small"
        variant="text"
        density="comfortable"
        :disabled="disabled"
        aria-label="Back to the suggestion"
        @click="emit('useSuggestion')"
      >
        <VIcon size="small">mdi-restore</VIcon>
        <VTooltip activator="parent" location="top" open-delay="300">Back to the suggestion</VTooltip>
      </VBtn>
    </template>
  </div>
</template>

<style scoped>
/* Wide enough for "Set by you · suggested: Non-combat" + ↺, so the column
   does not jump when the state changes. One line always: a grid track
   holding it grows to fit rather than wrap or clip. */
.facet-status {
  min-width: 260px;
  min-height: 28px;
  white-space: nowrap;
  /* `text-caption` no longer exists in Vuetify 4, so size the text here. */
  font-size: 0.8125rem;
}
.facet-status :deep(.v-btn) {
  font-size: 0.8125rem;
}
.status-text {
  white-space: nowrap;
}
</style>
