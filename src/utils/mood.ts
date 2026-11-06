import type { LibraryEntry, MoodScale, MoodStep } from '@generated/index';

// z-scores beyond this distance from the library mean leave the middle step.
const CUTOFF = 0.6;

/// The step shown for a mood scale: the confirmed step wins, else the step for
/// the analyzer's z-score, else null when neither exists.
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
  grand: ['Intimate', 'Moderate', 'Grand'],
  energy: ['Calm', 'Steady', 'Energetic'],
};

/// Row labels for each scale, as shown in the Scales panel and the tag scores panel.
export const SCALE_NAMES: Record<MoodScale, string> = {
  bright: 'Bright ↔ Dark',
  grand: 'Intimate ↔ Grand',
  energy: 'Calm ↔ Energetic',
};

export const STEPS: MoodStep[] = ['low', 'mid', 'high'];
export const MOOD_SCALES: MoodScale[] = ['bright', 'grand', 'energy'];

/// The entry's z-score and confirmed step for one scale.
export function scaleValues(entry: LibraryEntry, scale: MoodScale): { z: number | null; user: MoodStep | null } {
  switch (scale) {
    case 'bright':
      return { z: entry.moodBright, user: entry.moodBrightUser };
    case 'grand':
      return { z: entry.moodGrand, user: entry.moodGrandUser };
    case 'energy':
      return { z: entry.moodEnergy, user: entry.moodEnergyUser };
  }
}

/// The step shown for one scale of an entry.
export function entryStep(entry: LibraryEntry, scale: MoodScale): MoodStep | null {
  const { z, user } = scaleValues(entry, scale);
  return effectiveStep(z, user);
}
