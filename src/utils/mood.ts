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
  light: ['Heavy', 'Neutral', 'Airy'],
};

/// Row labels for each scale, as shown in the editor and the grid.
export const SCALE_NAMES: Record<MoodScale, string> = {
  bright: 'Bright ↔ Dark',
  light: 'Airy ↔ Heavy',
};

export const STEPS: MoodStep[] = ['low', 'mid', 'high'];

/// One square of the 3×3 mood grid: a Bright ↔ Dark step crossed with an
/// Airy ↔ Heavy step (the `light` scale; `high` is Airy).
export interface MoodSquare {
  key: string;
  bright: MoodStep;
  light: MoodStep;
  name: string;
  feel: string;
  scene: string;
}

/// Identifies a square by its two steps, e.g. "high-low" is Bright + Heavy.
export const squareKey = (bright: MoodStep, light: MoodStep): string => `${bright}-${light}`;

const square = (bright: MoodStep, light: MoodStep, name: string, feel: string, scene: string): MoodSquare =>
  ({ key: squareKey(bright, light), bright, light, name, feel, scene });

/// The nine squares in reading order: rows Bright → Dark, columns Airy → Heavy.
export const MOOD_SQUARES: MoodSquare[] = [
  square('high', 'high', 'Merry', 'Lute and fiddle, a carefree dance', 'Tavern night, village festival'),
  square('high', 'mid', 'Warm', 'Hopeful, open, an easy stride', 'Setting out, a friendly town'),
  square('high', 'low', 'Triumphant', 'Brass fanfare, swelling choir', "Heroes' charge, the dragon falls"),
  square('mid', 'high', 'Gentle', 'Calm, airy, drifting', 'Quiet camp, a meadow, a library'),
  square('mid', 'mid', 'Plain', 'Ambient, wandering, no strong pull', 'Exploring, travel'),
  square('mid', 'low', 'Weighty', 'Grand and serious, not menacing', 'The throne room, a council'),
  square('low', 'high', 'Melancholy', 'Lonely, sparse, aching', 'A grave, a farewell'),
  square('low', 'mid', 'Uneasy', "Tense, shadowed, something's off", 'A cave, a cursed forest'),
  square('low', 'low', 'Dread', 'Dense, crushing, menacing', 'Boss fight, the siege'),
];

/// The square for a pair of steps, or null when either step is missing.
export function squareFor(bright: MoodStep | null, light: MoodStep | null): MoodSquare | null {
  if (bright === null || light === null) return null;
  return MOOD_SQUARES.find((s) => s.bright === bright && s.light === light) ?? null;
}

/// The entry's square as shown (your steps win), or null when a scale has no value.
export function entrySquare(entry: LibraryEntry): MoodSquare | null {
  return squareFor(
    effectiveStep(entry.moodBright, entry.moodBrightUser),
    effectiveStep(entry.moodLight, entry.moodLightUser),
  );
}

export type PaceFilter = 'all' | Pace | 'unset';

export interface SoundFilters {
  /// Keys of the selected mood squares; empty means any mood.
  moods: string[];
  pace: PaceFilter;
}

export function emptySoundFilters(): SoundFilters {
  return { moods: [], pace: 'all' };
}

/// True when the entry passes the mood-square and Pace filters. With squares
/// selected, an entry passes when its shown square is one of them, so an entry
/// missing either mood value fails. An entry with no pace only passes "Any"
/// or "Unset".
export function matchesSoundFilters(entry: LibraryEntry, f: SoundFilters): boolean {
  if (f.moods.length > 0) {
    const sq = entrySquare(entry);
    if (sq === null || !f.moods.includes(sq.key)) return false;
  }
  if (f.pace === 'unset') return entry.entryType === 'music' && entry.pace === null;
  return f.pace === 'all' || entry.pace === f.pace;
}
