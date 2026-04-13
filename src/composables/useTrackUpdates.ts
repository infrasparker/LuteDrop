// All track field update functions — thin wrappers that build a partial update
// and call store.command('update_track', ...).

import type { Track, TrackMode, TrackTransition } from '@generated/index';
import { useAppStore } from '../stores/appStore';

export function useTrackUpdates(trackId: () => string, compId: () => string) {
  const store = useAppStore();

  async function updateField(updates: Partial<Track>): Promise<void> {
    await store.command('update_track', {
      compId: compId(),
      trackId: trackId(),
      updates,
    });
  }

  async function updateMode(type: string): Promise<void> {
    const track = store.findTrack(trackId());
    if (!track) return;

    let mode: TrackMode;
    if (type === 'seamless') {
      mode = { type: 'seamless', loopStart: 0, loopEnd: 0 };
    } else {
      const hasTransition = track.mode.type !== 'seamless';
      const transition = hasTransition
        ? (track.mode as { transition: TrackTransition }).transition
        : { type: 'gapless' as const };
      mode = { type: type as 'sequence' | 'shuffle', repeat: true, transition };
    }
    await updateField({ mode });
  }

  async function updateTransition(type: string): Promise<void> {
    const track = store.findTrack(trackId());
    if (!track) return;

    let transition: TrackTransition;
    switch (type) {
      case 'gap': transition = { type: 'gap', min: 0, max: 5 }; break;
      case 'crossfade': transition = { type: 'crossfade' }; break;
      default: transition = { type: 'gapless' }; break;
    }
    const mode = track.mode as { type: 'sequence' | 'shuffle'; repeat: boolean; transition: TrackTransition };
    await updateField({ mode: { ...mode, transition } });
  }

  function getSeamlessClipEntryId(): string | undefined {
    const track = store.findTrack(trackId());
    if (!track || track.clips.length === 0) return undefined;
    return track.clips[0].libraryEntryId;
  }

  function getCurrentLoopStart(): number {
    const track = store.findTrack(trackId());
    if (!track || track.mode.type !== 'seamless') return 0;
    return track.mode.loopStart;
  }

  function getCurrentLoopEnd(): number {
    const track = store.findTrack(trackId());
    if (!track || track.mode.type !== 'seamless') return 0;
    return track.mode.loopEnd;
  }

  async function updateLoopStart(v: string): Promise<void> {
    const entryId = getSeamlessClipEntryId();
    if (!entryId) return;
    await store.command('update_library_entry_loop_points', {
      entryId,
      loopStart: Number(v),
      loopEnd: getCurrentLoopEnd(),
    });
  }

  async function updateLoopEnd(v: string): Promise<void> {
    const entryId = getSeamlessClipEntryId();
    if (!entryId) return;
    await store.command('update_library_entry_loop_points', {
      entryId,
      loopStart: getCurrentLoopStart(),
      loopEnd: Number(v),
    });
  }

  async function updateLoopStartFromWaveform(v: number): Promise<void> {
    const entryId = getSeamlessClipEntryId();
    if (!entryId) return;
    await store.command('update_library_entry_loop_points', {
      entryId,
      loopStart: v,
      loopEnd: getCurrentLoopEnd(),
    });
  }

  async function updateLoopEndFromWaveform(v: number): Promise<void> {
    const entryId = getSeamlessClipEntryId();
    if (!entryId) return;
    await store.command('update_library_entry_loop_points', {
      entryId,
      loopStart: getCurrentLoopStart(),
      loopEnd: v,
    });
  }

  async function updateRepeat(v: boolean): Promise<void> {
    const track = store.findTrack(trackId());
    if (!track) return;
    const mode = track.mode as any;
    await updateField({ mode: { ...mode, repeat: v } });
  }

  async function updateGapMin(v: string): Promise<void> {
    const track = store.findTrack(trackId());
    if (!track) return;
    const mode = track.mode as any;
    await updateField({ mode: { ...mode, transition: { type: 'gap', min: Number(v), max: mode.transition.max } } });
  }

  async function updateGapMax(v: string): Promise<void> {
    const track = store.findTrack(trackId());
    if (!track) return;
    const mode = track.mode as any;
    await updateField({ mode: { ...mode, transition: { type: 'gap', min: mode.transition.min, max: Number(v) } } });
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
    updateLoopStart,
    updateLoopEnd,
    updateLoopStartFromWaveform,
    updateLoopEndFromWaveform,
    updateRepeat,
    updateGapMin,
    updateGapMax,
    toggleFadeIn,
    toggleFadeOut,
  };
}
