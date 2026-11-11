<script setup lang="ts">
import { computed, ref } from 'vue';
import { useAppStore } from '../stores/appStore';
import { useSoundstageUiStore } from '../stores/soundstageUi';
import { playbackLabel, trackClipSummary, trackLocations, trackSearchText } from '../utils/soundstage';
import TrackPickerDialog from '../components/TrackPickerDialog.vue';
const store = useAppStore();
const ui = useSoundstageUiStore();
const search = ref('');
const showNew = ref(false);
const tracks = computed(() => store.tracks.filter((track) => trackSearchText(track, store.boards).includes((search.value ?? '').trim().normalize('NFKC').toLocaleLowerCase())));
</script>
<template>
  <div class="tracks-view">
    <Teleport to="#app-bar-main"><span class="text-subtitle-2">Tracks</span><VSpacer /><VBtn text="New track" prepend-icon="mdi-plus" variant="tonal" rounded="xl" size="small" @click="showNew = true" /></Teleport>
    <div class="inventory-heading"><div><h1>Track inventory</h1><p>Reusable playback layers. Clips and playback settings are shared; scenes remember their own mix.</p></div><VTextField v-model="search" label="Find a track" prepend-inner-icon="mdi-magnify" variant="outlined" density="compact" hide-details clearable @click:clear="search = ''" /></div>
    <div class="inventory-list"><button v-for="track in tracks" :key="track.id" class="inventory-track" @click="ui.editTrackId = track.id"><VIcon class="inventory-icon">mdi-tune-vertical</VIcon><span class="inventory-name">{{ track.name }}<small>{{ track.clips.length }} {{ track.clips.length === 1 ? 'clip' : 'clips' }} · {{ playbackLabel(track.mode) }}</small><span class="inventory-clips" :title="track.clips.map((clip) => clip.name).join(' · ')">{{ trackClipSummary(track) }}</span></span><span class="inventory-context"><small>This configuration</small>{{ trackLocations(track.id, store.boards).join(' · ') || 'Not on a board here' }}</span><span class="inventory-edit">Edit track <VIcon size="16">mdi-chevron-right</VIcon></span></button></div>
    <div v-if="!store.tracks.length" class="inventory-empty"><VIcon size="40">mdi-tune-vertical</VIcon><h2>Your tracks live here, with or without scenes</h2><p>Create a track from files in the Library, then reuse it on your boards.</p><VBtn text="Create track" prepend-icon="mdi-plus" variant="tonal" color="primary" class="mt-4" @click="showNew = true" /></div>
    <p v-else-if="!tracks.length" class="text-medium-emphasis mt-5">No tracks match “{{ search }}”.</p>
    <TrackPickerDialog v-model="showNew" create-only />
  </div>
</template>
<style scoped>
.tracks-view { padding: 28px; height: 100%; overflow-y: auto; }
.inventory-heading { display: flex; align-items: center; gap: 32px; justify-content: space-between; margin-bottom: 24px; }
.inventory-heading h1 { font-size: 22px; font-weight: 500; }
.inventory-heading p { margin-top: 6px; opacity: .6; font-size: 13px; max-width: 600px; }
.inventory-heading :deep(.v-input) { flex: 0 0 300px; }
.inventory-list { border-top: 1px solid rgba(var(--v-theme-on-surface), .12); }
.inventory-track { display: flex; width: 100%; text-align: left; gap: 16px; align-items: center; color: inherit; padding: 18px 12px; border: 0; border-bottom: 1px solid rgba(var(--v-theme-on-surface), .1); background: transparent; cursor: pointer; }
.inventory-track:hover { background: rgba(var(--v-theme-on-surface), .04); }
.inventory-track:focus-visible { outline: 2px solid rgb(var(--v-theme-primary)); }
.inventory-icon { opacity: .5; }
.inventory-name { flex: 1; min-width: 0; font-size: 14px; overflow-wrap: anywhere; }
.inventory-name small { display: block; opacity: .55; font-size: 12px; margin-top: 5px; }
.inventory-clips { display: block; margin-top: 7px; font-size: 12px; opacity: .75; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.inventory-context { flex: 0 1 32%; min-width: 160px; max-width: 420px; font-size: 12px; opacity: .7; line-height: 1.6; overflow-wrap: anywhere; }
.inventory-context small { display: block; margin-bottom: 3px; font-size: 11px; opacity: .65; }
.inventory-edit { font-size: 12px; opacity: .6; }
.inventory-empty { text-align: center; padding: 64px 24px; }
.inventory-empty h2 { font-size: 20px; margin-top: 18px; font-weight: 500; }
.inventory-empty p { opacity: .6; margin-top: 10px; }
@media(max-width:800px) { .inventory-heading { align-items: stretch; flex-direction: column; gap: 16px; } .inventory-heading :deep(.v-input) { flex-basis: auto; } .inventory-track { flex-wrap: wrap; } .inventory-name { flex-basis: calc(100% - 155px); } .inventory-context { margin-left: 40px; flex-basis: calc(100% - 40px); max-width: none; order: 3; } }
</style>
