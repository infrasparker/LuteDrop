import type { LibraryEntry, MoodScale, MoodStep } from '@generated/index';

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
  grand: ['Intimate', 'Moderate', 'Grand'],
  energy: ['Calm', 'Steady', 'Energetic'],
};

/// Row labels for each scale, as shown in the editor and the tag scores panel.
export const SCALE_NAMES: Record<MoodScale, string> = {
  bright: 'Bright ↔ Dark',
  grand: 'Intimate ↔ Grand',
  energy: 'Calm ↔ Energetic',
};

export const STEPS: MoodStep[] = ['low', 'mid', 'high'];
export const MOOD_SCALES: MoodScale[] = ['bright', 'grand', 'energy'];

/// The label of one step on one scale, e.g. stepLabel('energy', 'high') = "Energetic".
export const stepLabel = (scale: MoodScale, step: MoodStep): string => STEP_LABELS[scale][STEPS.indexOf(step)];

/// Pad layout: sizes are the floors (Intimate → Grand), rows run Energetic on
/// top down to Calm, columns run Dark → Bright.
export const PAD_SIZES: MoodStep[] = ['low', 'mid', 'high'];
export const PAD_ROWS: MoodStep[] = ['high', 'mid', 'low'];
export const PAD_COLUMNS: MoodStep[] = ['low', 'mid', 'high'];

/// The three steps that place a track in the mood pad.
export interface MoodSteps {
  grand: MoodStep;
  energy: MoodStep;
  bright: MoodStep;
}

/// One cell of the mood pad: a size (floor), an energy row and a brightness
/// column, named with a few words of feel and an example scene.
export interface MoodCell extends MoodSteps {
  key: string;
  name: string;
  feel: string;
  scene: string;
}

/// Identifies a cell by its steps, "<grand>-<energy>-<bright>",
/// e.g. "low-high-high" is Intimate + Energetic + Bright (Merry).
export const moodKey = (grand: MoodStep, energy: MoodStep, bright: MoodStep): string => `${grand}-${energy}-${bright}`;

const cell = (
  grand: MoodStep,
  energy: MoodStep,
  bright: MoodStep,
  name: string,
  feel: string,
  scene: string,
): MoodCell => ({ key: moodKey(grand, energy, bright), grand, energy, bright, name, feel, scene });

/// All 27 cells, floor by floor (Intimate, Moderate, Grand), each floor in
/// reading order: rows Energetic → Calm, columns Dark → Bright.
export const MOOD_CELLS: MoodCell[] = [
  // Intimate
  cell('low', 'high', 'low', 'Anxious', 'Skittering, nervous, on edge', 'A pickpocket chase, a trap springs'),
  cell('low', 'high', 'mid', 'Spirited', 'Quick, plucky, light-footed', 'A rooftop dash, a card game'),
  cell('low', 'high', 'high', 'Merry', 'Lute and fiddle, carefree', 'A tavern jig, village festival'),
  cell('low', 'mid', 'low', 'Lonely', 'Sparse, aching, alone', 'A long road, a ruined chapel'),
  cell('low', 'mid', 'mid', 'Wandering', 'Unhurried, curious, meandering', 'Back roads, a quiet market'),
  cell('low', 'mid', 'high', 'Whimsical', 'A music box, playful', "A fey glade, a gnome's workshop"),
  cell('low', 'low', 'low', 'Sorrowful', 'Slow lament, grieving', 'A grave, a farewell'),
  cell('low', 'low', 'mid', 'Nostalgic', 'Soft, wistful, remembering', 'An old home, a letter from afar'),
  cell('low', 'low', 'high', 'Tender', 'Warm, gentle, close', 'A campfire talk, a reunion'),
  // Moderate
  cell('mid', 'high', 'low', 'Tense', 'Pulsing, shadowed, closing in', 'The ambush, a dark chase'),
  cell('mid', 'high', 'mid', 'Urgent', 'Fast, purposeful, no time', 'Racing the clock, a pursuit'),
  cell('mid', 'high', 'high', 'Lively', 'Bouncing, bright, full of life', 'Market day, a summer fair'),
  cell('mid', 'mid', 'low', 'Uneasy', "Shadowed, something's off", 'A cave, a cursed forest'),
  cell('mid', 'mid', 'mid', 'Exploring', 'Ambient, steady, no strong pull', 'Dungeon halls, travel'),
  cell('mid', 'mid', 'high', 'Hopeful', 'Open, warm, an easy stride', 'Setting out, a friendly town'),
  cell('mid', 'low', 'low', 'Somber', 'Grave, slow, grey', 'The aftermath, a wake'),
  cell('mid', 'low', 'mid', 'Meditative', 'Quiet, drifting, unhurried', 'A shrine, a library, long rest'),
  cell('mid', 'low', 'high', 'Peaceful', 'Calm, warm, at ease', 'Quiet camp, a meadow'),
  // Grand
  cell('high', 'high', 'low', 'Ferocious', 'Pounding drums, brass, menace', 'Boss fight, the siege'),
  cell('high', 'high', 'mid', 'Powerful', 'Driving, mighty, full orchestra', 'A great battle, the clash'),
  cell('high', 'high', 'high', 'Triumphant', 'Brass fanfare, swelling choir', "Heroes' charge, the dragon falls"),
  cell('high', 'mid', 'low', 'Menacing', 'Low brass, a looming threat', "The villain's lair, an army marches"),
  cell('high', 'mid', 'mid', 'Stately', 'Measured, formal, dignified', 'The throne room, a council'),
  cell('high', 'mid', 'high', 'Majestic', 'Soaring, noble, sweeping', 'A royal city, the coronation'),
  cell('high', 'low', 'low', 'Ominous', 'Vast, dark, foreboding', "A dead god's tomb, the abyss"),
  cell('high', 'low', 'mid', 'Solemn', 'Slow choir, reverent', "A cathedral, a hero's funeral"),
  cell('high', 'low', 'high', 'Wondrous', 'Shimmering, vast, awestruck', 'An elven city, a starlit sky'),
];

const CELLS_BY_KEY = new Map(MOOD_CELLS.map((c) => [c.key, c]));

export const cellByKey = (key: string): MoodCell | null => CELLS_BY_KEY.get(key) ?? null;

/// The cell for three steps, or null when any step is missing.
export function cellFor(grand: MoodStep | null, energy: MoodStep | null, bright: MoodStep | null): MoodCell | null {
  if (grand === null || energy === null || bright === null) return null;
  return cellByKey(moodKey(grand, energy, bright));
}

/// The entry's cell as shown (your steps win), or null when a scale has no value.
export function entryCell(entry: LibraryEntry): MoodCell | null {
  return cellFor(
    effectiveStep(entry.moodGrand, entry.moodGrandUser),
    effectiveStep(entry.moodEnergy, entry.moodEnergyUser),
    effectiveStep(entry.moodBright, entry.moodBrightUser),
  );
}

/// The cell the analyzer alone would put the entry in, ignoring your steps.
export function entryAutoCell(entry: LibraryEntry): MoodCell | null {
  return cellFor(
    effectiveStep(entry.moodGrand, null),
    effectiveStep(entry.moodEnergy, null),
    effectiveStep(entry.moodBright, null),
  );
}

/// The entry's z-score and your step for one scale.
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

/// How many entries sit in each cell, keyed by cell key. Entries that are not
/// placed (a scale has no value) are not counted.
export function countByCell(entries: LibraryEntry[]): Map<string, number> {
  const counts = new Map<string, number>();
  for (const entry of entries) {
    const c = entryCell(entry);
    if (c) counts.set(c.key, (counts.get(c.key) ?? 0) + 1);
  }
  return counts;
}

export interface SoundFilters {
  /// Keys of the selected mood cells ("<grand>-<energy>-<bright>"); empty means any mood.
  moods: string[];
}

export function emptySoundFilters(): SoundFilters {
  return { moods: [] };
}

/// True when the entry passes the mood filter. With cells selected, an entry
/// passes when its shown cell is one of them, so an entry missing any of the
/// three mood values fails.
export function matchesSoundFilters(entry: LibraryEntry, f: SoundFilters): boolean {
  if (f.moods.length === 0) return true;
  const c = entryCell(entry);
  return c !== null && f.moods.includes(c.key);
}
