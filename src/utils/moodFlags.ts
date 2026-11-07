// Pure helpers for flagging a mood bar that looks wrong (see MoodProfile.vue).
import type { MoodFlag } from '@generated/index';

/// The bands a flag can name, in the order the menu lists them.
export const FLAG_BANDS = [
  { band: 'definitely', label: 'Definitely' },
  { band: 'understandable', label: 'Understandable' },
  { band: 'stretch', label: 'Stretch' },
  { band: 'no', label: 'No' },
] as const;

const BAND_LABEL: Record<string, string> = Object.fromEntries(FLAG_BANDS.map((b) => [b.band, b.label]));

/// A flag in words: "Definitely", "At least Understandable", "Higher", ...
/// A flag made under older mood wording says so.
export function flagLabel(flag: Pick<MoodFlag, 'kind' | 'band' | 'bound' | 'stale'>): string {
  let text: string;
  if (flag.kind === 'higher') text = 'Higher';
  else if (flag.kind === 'lower') text = 'Lower';
  else {
    const band = BAND_LABEL[flag.band ?? ''] ?? '';
    text = flag.bound === 'at_least' ? `At least ${band}` : flag.bound === 'at_most' ? `At most ${band}` : band;
  }
  return flag.stale ? `${text} · made under older wording` : text;
}

/// Flagging is for one song at a time, and Energy is not a mood to flag.
export function flagControlVisible(entryCount: number, moodId: string): boolean {
  return entryCount <= 1 && moodId !== 'energy';
}
