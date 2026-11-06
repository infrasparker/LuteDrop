import type { LibraryEntry, MoodStep, Pool } from '@generated/index';
import { entryStep } from './mood';
import { effectivePool } from './pools';

/// One word that sums up a track on the Review card: its pool, shaded by its
/// energy step and its size step. It is only a readout: never stored and never
/// used to filter.
///
/// Layout per pool: rows are energy (Calm, Steady, Energetic), columns are
/// size (Intimate, Moderate, Grand). 7 pools × 3 × 3 = 63 words, all distinct.
type Grid = Record<MoodStep, [string, string, string]>;

export const READOUT: Record<Pool, Grid> = {
  peaceful: {
    low: ['Cozy', 'Serene', 'Spacious'],
    mid: ['Gentle', 'Easygoing', 'Idyllic'],
    high: ['Breezy', 'Flowing', 'Sunlit'],
  },
  lighthearted: {
    low: ['Quaint', 'Whimsical', 'Charming'],
    mid: ['Playful', 'Jaunty', 'Festive'],
    high: ['Rollicking', 'Merry', 'Jubilant'],
  },
  somber: {
    low: ['Sorrowful', 'Mournful', 'Solemn'],
    mid: ['Wistful', 'Melancholy', 'Lamenting'],
    high: ['Aching', 'Anguished', 'Tragic'],
  },
  awe: {
    low: ['Reverent', 'Hallowed', 'Wondrous'],
    mid: ['Hopeful', 'Uplifting', 'Majestic'],
    high: ['Eager', 'Soaring', 'Glorious'],
  },
  mysterious: {
    low: ['Uncanny', 'Eerie', 'Cavernous'],
    mid: ['Creeping', 'Ominous', 'Foreboding'],
    high: ['Skittering', 'Frantic', 'Nightmarish'],
  },
  epic: {
    low: ['Resolute', 'Steadfast', 'Stately'],
    mid: ['Determined', 'Marching', 'Heroic'],
    high: ['Scrappy', 'Fierce', 'Triumphant'],
  },
  tension: {
    low: ['Uneasy', 'Watchful', 'Brooding'],
    mid: ['Nervous', 'Looming', 'Menacing'],
    high: ['Hunted', 'Urgent', 'Relentless'],
  },
};

const SIZE_INDEX: Record<MoodStep, number> = { low: 0, mid: 1, high: 2 };

/// The word for a pool, energy step and size step. A missing step counts as
/// the middle one (Steady, Moderate).
export function readoutWord(pool: Pool, energy: MoodStep | null, size: MoodStep | null): string {
  return READOUT[pool][energy ?? 'mid'][SIZE_INDEX[size ?? 'mid']];
}

/// The entry's word from its shown pool and steps, or null when it has no pool.
export function entryReadout(entry: LibraryEntry): string | null {
  const pool = effectivePool(entry);
  if (pool === null) return null;
  return readoutWord(pool, entryStep(entry, 'energy'), entryStep(entry, 'grand'));
}
