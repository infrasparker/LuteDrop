import type { DarkKind, LibraryEntry, MoodScale, MoodStep } from '@generated/index';

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

/// The two kinds of dark music, and their labels. A track's kind only shows
/// when its Bright ↔ Dark step is Dark.
export const DARK_KINDS: DarkKind[] = ['sad', 'scary'];
export const DARK_KIND_LABELS: Record<DarkKind, string> = { sad: 'Sad', scary: 'Scary' };

/// One column of the mood pad: a Bright ↔ Dark step, plus the dark kind for
/// the two Dark sub-columns (null for Neutral and Bright).
export interface PadColumn {
  bright: MoodStep;
  kind: DarkKind | null;
  label: string;
}

/// Pad layout: sizes are the floors (Intimate → Grand), rows run Energetic on
/// top down to Calm, columns run Dark (Sad, Scary) → Neutral → Bright.
export const PAD_SIZES: MoodStep[] = ['low', 'mid', 'high'];
export const PAD_ROWS: MoodStep[] = ['high', 'mid', 'low'];
export const PAD_COLUMNS: PadColumn[] = [
  { bright: 'low', kind: 'sad', label: 'Sad' },
  { bright: 'low', kind: 'scary', label: 'Scary' },
  { bright: 'mid', kind: null, label: 'Neutral' },
  { bright: 'high', kind: null, label: 'Bright' },
];

/// The three steps that place a track in the mood pad.
export interface MoodSteps {
  grand: MoodStep;
  energy: MoodStep;
  bright: MoodStep;
}

/// One cell of the mood pad: a size (floor), an energy row and a brightness
/// column, named with a few words of feel and an example scene. Dark cells
/// also carry a kind (Sad or Scary); the other cells have kind null.
export interface MoodCell extends MoodSteps {
  kind: DarkKind | null;
  key: string;
  name: string;
  feel: string;
  scene: string;
}

/// Identifies a place in the pad by its steps, "<grand>-<energy>-<bright>",
/// e.g. "low-high-high" is Intimate + Energetic + Bright (Merry). A Dark
/// place with a kind adds it: "mid-low-low-scary" is Moderate + Calm + Dark,
/// Scary (Eerie). A Dark place with no kind ("mid-low-low") is not a cell.
export const moodKey = (grand: MoodStep, energy: MoodStep, bright: MoodStep, kind: DarkKind | null = null): string =>
  bright === 'low' && kind !== null ? `${grand}-${energy}-${bright}-${kind}` : `${grand}-${energy}-${bright}`;

const cell = (
  grand: MoodStep,
  energy: MoodStep,
  bright: MoodStep,
  name: string,
  feel: string,
  scene: string,
): MoodCell => ({ key: moodKey(grand, energy, bright), grand, energy, bright, kind: null, name, feel, scene });

/// The six dark moods. Their names do not depend on size, so the words do not
/// change when the size switch does; each one appears on all three floors.
const DARK_MOODS: { energy: MoodStep; kind: DarkKind; name: string; feel: string; scene: string }[] = [
  { energy: 'high', kind: 'sad', name: 'Desperate', feel: 'Racing, aching, all is lost', scene: 'A last stand, the city falls' },
  { energy: 'high', kind: 'scary', name: 'Harrowing', feel: 'Pounding, relentless, terror', scene: 'A chase in the dark, a boss fight' },
  { energy: 'mid', kind: 'sad', name: 'Mournful', feel: 'Grave, aching, weighed down', scene: 'The aftermath, a wake' },
  { energy: 'mid', kind: 'scary', name: 'Menacing', feel: 'Low pulse, a looming threat', scene: "The villain's lair, a cursed forest" },
  { energy: 'low', kind: 'sad', name: 'Sorrowful', feel: 'Slow lament, grieving', scene: 'A grave, a farewell' },
  { energy: 'low', kind: 'scary', name: 'Eerie', feel: 'Hushed, creeping, uncanny', scene: 'A heartbeat in the dark, a crypt' },
];

/// The Neutral and Bright cells, whose names change with size: floor by
/// floor (Intimate, Moderate, Grand), rows Energetic → Calm, Neutral then Bright.
const NEUTRAL_AND_BRIGHT_CELLS: MoodCell[] = [
  // Intimate
  cell('low', 'high', 'mid', 'Spirited', 'Quick, plucky, light-footed', 'A rooftop dash, a card game'),
  cell('low', 'high', 'high', 'Merry', 'Lute and fiddle, carefree', 'A tavern jig, village festival'),
  cell('low', 'mid', 'mid', 'Wandering', 'Unhurried, curious, meandering', 'Back roads, a quiet market'),
  cell('low', 'mid', 'high', 'Whimsical', 'A music box, playful', "A fey glade, a gnome's workshop"),
  cell('low', 'low', 'mid', 'Nostalgic', 'Soft, wistful, remembering', 'An old home, a letter from afar'),
  cell('low', 'low', 'high', 'Tender', 'Warm, gentle, close', 'A campfire talk, a reunion'),
  // Moderate
  cell('mid', 'high', 'mid', 'Urgent', 'Fast, purposeful, no time', 'Racing the clock, a pursuit'),
  cell('mid', 'high', 'high', 'Lively', 'Bouncing, bright, full of life', 'Market day, a summer fair'),
  cell('mid', 'mid', 'mid', 'Exploring', 'Ambient, steady, no strong pull', 'Dungeon halls, travel'),
  cell('mid', 'mid', 'high', 'Hopeful', 'Open, warm, an easy stride', 'Setting out, a friendly town'),
  cell('mid', 'low', 'mid', 'Meditative', 'Quiet, drifting, unhurried', 'A shrine, a library, long rest'),
  cell('mid', 'low', 'high', 'Peaceful', 'Calm, warm, at ease', 'Quiet camp, a meadow'),
  // Grand
  cell('high', 'high', 'mid', 'Powerful', 'Mighty, surging, full orchestra', 'A great battle, the clash'),
  cell('high', 'high', 'high', 'Triumphant', 'Brass fanfare, swelling choir', "Heroes' charge, the dragon falls"),
  cell('high', 'mid', 'mid', 'Stately', 'Measured, formal, dignified', 'The throne room, a council'),
  cell('high', 'mid', 'high', 'Majestic', 'Soaring, noble, sweeping', 'A royal city, the coronation'),
  cell('high', 'low', 'mid', 'Solemn', 'Slow choir, reverent', "A cathedral, a hero's funeral"),
  cell('high', 'low', 'high', 'Wondrous', 'Shimmering, vast, awestruck', 'An elven city, a starlit sky'),
];

/// All 36 cells: on each floor, 6 dark ones (3 rows × Sad, Scary) and 6 others.
export const MOOD_CELLS: MoodCell[] = [
  ...PAD_SIZES.flatMap((grand) =>
    DARK_MOODS.map(({ energy, kind, name, feel, scene }): MoodCell => ({
      key: moodKey(grand, energy, 'low', kind),
      grand,
      energy,
      bright: 'low',
      kind,
      name,
      feel,
      scene,
    })),
  ),
  ...NEUTRAL_AND_BRIGHT_CELLS,
];

const CELLS_BY_KEY = new Map(MOOD_CELLS.map((c) => [c.key, c]));

export const cellByKey = (key: string): MoodCell | null => CELLS_BY_KEY.get(key) ?? null;

/// The cell for three steps (and a kind, for Dark), or null when any step is
/// missing or the place is Dark with no kind.
export function cellFor(
  grand: MoodStep | null,
  energy: MoodStep | null,
  bright: MoodStep | null,
  kind: DarkKind | null = null,
): MoodCell | null {
  if (grand === null || energy === null || bright === null) return null;
  return cellByKey(moodKey(grand, energy, bright, kind));
}

/// Where a track sits in the pad: its three steps, plus its dark kind when it
/// is Dark. `cell` is null for a Dark track with no kind yet (no Sad/Scary
/// scores and none of yours); such a track sits on the Dark header.
export interface MoodPlace extends MoodSteps {
  kind: DarkKind | null;
  key: string;
  cell: MoodCell | null;
}

function placeFor(
  grand: MoodStep | null,
  energy: MoodStep | null,
  bright: MoodStep | null,
  kind: DarkKind | null,
): MoodPlace | null {
  if (grand === null || energy === null || bright === null) return null;
  // A kind only counts on Dark; elsewhere it is stored but not shown.
  const k = bright === 'low' ? kind : null;
  return { grand, energy, bright, kind: k, key: moodKey(grand, energy, bright, k), cell: cellFor(grand, energy, bright, k) };
}

/// The name of a place: its cell's mood, or "Dark" for a Dark track with no kind.
export const placeName = (p: MoodPlace): string => p.cell?.name ?? 'Dark';

/// The dark kind shown for an entry: yours, else the analyzer's.
export const entryDarkKind = (entry: LibraryEntry): DarkKind | null => entry.darkKindUser ?? entry.darkKindAuto;

/// The entry's place as shown (your steps and kind win), or null when a scale has no value.
export function entryPlace(entry: LibraryEntry): MoodPlace | null {
  return placeFor(
    effectiveStep(entry.moodGrand, entry.moodGrandUser),
    effectiveStep(entry.moodEnergy, entry.moodEnergyUser),
    effectiveStep(entry.moodBright, entry.moodBrightUser),
    entryDarkKind(entry),
  );
}

/// The place the analyzer alone would give the entry, ignoring your steps and kind.
export function entryAutoPlace(entry: LibraryEntry): MoodPlace | null {
  return placeFor(
    effectiveStep(entry.moodGrand, null),
    effectiveStep(entry.moodEnergy, null),
    effectiveStep(entry.moodBright, null),
    entry.darkKindAuto,
  );
}

/// The entry's cell as shown, or null when a scale has no value or the entry
/// is Dark with no kind.
export const entryCell = (entry: LibraryEntry): MoodCell | null => entryPlace(entry)?.cell ?? null;

/// True when the entry's shown Bright ↔ Dark step is Dark: the only case
/// where its dark kind is shown.
export const isShownDark = (entry: LibraryEntry): boolean =>
  effectiveStep(entry.moodBright, entry.moodBrightUser) === 'low';

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
/// in a cell (a scale has no value, or Dark with no kind) are not counted.
export function countByCell(entries: LibraryEntry[]): Map<string, number> {
  const counts = new Map<string, number>();
  for (const entry of entries) {
    const c = entryCell(entry);
    if (c) counts.set(c.key, (counts.get(c.key) ?? 0) + 1);
  }
  return counts;
}

export interface SoundFilters {
  /// Keys of the selected mood cells (see `moodKey`); empty means any mood.
  moods: string[];
}

export function emptySoundFilters(): SoundFilters {
  return { moods: [] };
}

/// True when the entry passes the mood filter. With cells selected, an entry
/// passes when its shown cell is one of them, so an entry missing any of the
/// three mood values fails, and so does a Dark entry with no kind.
export function matchesSoundFilters(entry: LibraryEntry, f: SoundFilters): boolean {
  if (f.moods.length === 0) return true;
  const c = entryCell(entry);
  return c !== null && f.moods.includes(c.key);
}
