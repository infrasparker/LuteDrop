// All track field update functions — thin wrappers that build a partial update
// and call store.command('update_track', ...).

import type { Track, TrackMode, TrackTransition } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import { hasTransition } from '../shared/trackMode';

export function useTrackUpdates(trackId: () => string) {
  const store = useAppStore();

  async function updateField(updates: Partial<Track>): Promise<void> {
    await store.command('update_track', {
      trackId: trackId(),
      updates,
    });
  }

  async function updateMode(type: string): Promise<void> {
    const track = store.findTrack(trackId());
    if (!track) return;
    let mode: TrackMode;
    if (type === 'oneShot') {
      mode = { type: 'oneShot' };
    } else if (type === 'seamless') {
      mode = { type: 'seamless', loopStart: 0, loopEnd: 0 };
    } else {
      const transition = hasTransition(track.mode) ? track.mode.transition : { type: 'gapless' as const };
      mode = { type: type as 'sequence' | 'shuffle', repeat: true, transition };
    }
    await updateField({ mode });
  }

  async function updateTransition(type: string): Promise<void> {
    const track = store.findTrack(trackId());
    if (!track || !hasTransition(track.mode)) return;
    let transition: TrackTransition;
    switch (type) {
      case 'gap': transition = { type: 'gap', min: 0, max: 5 }; break;
      case 'crossfade': transition = { type: 'crossfade' }; break;
      default: transition = { type: 'gapless' }; break;
    }
    await updateField({ mode: { ...track.mode, transition } });
  }

  async function updateRepeat(v: boolean | null): Promise<void> {
    const track = store.findTrack(trackId());
    if (!track || !hasTransition(track.mode)) return;
    await updateField({ mode: { ...track.mode, repeat: !!v } });
  }

  async function updateGapMin(v: string): Promise<void> {
    const track = store.findTrack(trackId());
    if (!track || !hasTransition(track.mode)) return;
    const t = track.mode.transition;
    const max = t.type === 'gap' ? t.max : 5;
    await updateField({ mode: { ...track.mode, transition: { type: 'gap', min: Number(v), max } } });
  }

  async function updateGapMax(v: string): Promise<void> {
    const track = store.findTrack(trackId());
    if (!track || !hasTransition(track.mode)) return;
    const t = track.mode.transition;
    const min = t.type === 'gap' ? t.min : 0;
    await updateField({ mode: { ...track.mode, transition: { type: 'gap', min, max: Number(v) } } });
  }

  async function toggleFadeIn(v: boolean | null): Promise<void> {
    await updateField({ fadeIn: v ? 1 : 0 });
  }

  async function toggleFadeOut(v: boolean | null): Promise<void> {
    await updateField({ fadeOut: v ? 1 : 0 });
  }

  return {
    updateField,
    updateMode,
    updateTransition,
    updateRepeat,
    updateGapMin,
    updateGapMax,
    toggleFadeIn,
    toggleFadeOut,
  };
}
