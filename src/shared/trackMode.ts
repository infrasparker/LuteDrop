import type { TrackMode } from '@generated/index';

type SeamlessMode = Extract<TrackMode, { type: 'seamless' }>;
type SequencedMode = Extract<TrackMode, { type: 'sequence' | 'shuffle' }>;

/// Narrows a TrackMode to its Seamless variant.
export function isSeamless(mode: TrackMode): mode is SeamlessMode {
  return mode.type === 'seamless';
}

/// Narrows a TrackMode to a Sequence/Shuffle variant (which carry repeat + transition).
export function hasTransition(mode: TrackMode): mode is SequencedMode {
  return mode.type === 'sequence' || mode.type === 'shuffle';
}
