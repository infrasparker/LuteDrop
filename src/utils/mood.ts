import type { LibraryEntry, MoodScale, MoodStep, Pace } from '@generated/index';

// z-scores beyond this distance from the library mean leave the middle step.
const CUTOFF = 0.6;

/// The step shown for a mood scale: the user's step wins, else the step for the
/// analyzer's z-score, else null when neither exists.
export function effectiveStep(z: number | null, user: MoodStep | null): MoodStep | null {
  if (user !== null) return user;
  if (z === null) return null;
  if (z < -CUTOFF) return 'low';
  if (z > CUTOFF) return 'high';
  return 'mid';
}

/// Button labels per scale, indexed low / mid / high.
export const STEP_LABELS: Record<MoodScale, [string, string, string]> = {
  bright: ['Dark', 'Neutral', 'Bright'],
  light: ['Heavy', 'Neutral', 'Light'],
};

export const STEPS: MoodStep[] = ['low', 'mid', 'high'];

export type StepFilter = 'all' | MoodStep;
export type PaceFilter = 'all' | Pace | 'unset';

export interface SoundFilters {
  bright: StepFilter;
  light: StepFilter;
  pace: PaceFilter;
}

export function emptySoundFilters(): SoundFilters {
  return { bright: 'all', light: 'all', pace: 'all' };
}

/// True when the entry passes the Bright/Dark, Light/Heavy and Pace filters.
/// An entry with no mood or pace value only passes "Any" (or "Unset" for pace).
export function matchesSoundFilters(entry: LibraryEntry, f: SoundFilters): boolean {
  if (f.bright !== 'all' && effectiveStep(entry.moodBright, entry.moodBrightUser) !== f.bright) {
    return false;
  }
  if (f.light !== 'all' && effectiveStep(entry.moodLight, entry.moodLightUser) !== f.light) {
    return false;
  }
  if (f.pace === 'unset') return entry.entryType === 'music' && entry.pace === null;
  return f.pace === 'all' || entry.pace === f.pace;
}
