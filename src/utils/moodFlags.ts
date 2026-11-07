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
/// A stale flag says why: made under older mood wording, or (a higher /
/// lower flag) under an older calibration of its mood.
export function flagLabel(flag: Pick<MoodFlag, 'kind' | 'band' | 'bound' | 'stale' | 'staleReason'>): string {
  let text: string;
  if (flag.kind === 'higher') text = 'Higher';
  else if (flag.kind === 'lower') text = 'Lower';
  else {
    const band = BAND_LABEL[flag.band ?? ''] ?? '';
    text = flag.bound === 'at_least' ? `At least ${band}` : flag.bound === 'at_most' ? `At most ${band}` : band;
  }
  if (!flag.stale) return text;
  return flag.staleReason === 'calibration'
    ? `${text} · made under an older calibration`
    : `${text} · made under older wording`;
}

/// The band a hand-set level stands for, as Tune reads it: 1 → Definitely,
/// 0.75 → Understandable, 0.5 → Stretch, 0 or 0.25 → No.
export function levelBand(level: number): (typeof FLAG_BANDS)[number]['band'] {
  if (level >= 1) return 'definitely';
  if (level >= 0.75) return 'understandable';
  if (level >= 0.5) return 'stretch';
  return 'no';
}

/// What a Tune review row's song is graded against now: its flag, else its
/// hand-set level ("Hand-set: Understandable"), else "flag removed".
export function reviewTargetLabel(
  flag: Pick<MoodFlag, 'kind' | 'band' | 'bound' | 'stale' | 'staleReason'> | null,
  level: number | null,
): string {
  if (flag) return flagLabel(flag);
  if (level !== null) return `Hand-set: ${BAND_LABEL[levelBand(level)]}`;
  return 'flag removed';
}

/// The short mark beside a flagged bar's icon: the band's initial (D / U /
/// S / N) or an arrow for higher / lower.
export function flagMark(flag: Pick<MoodFlag, 'kind' | 'band'>): string {
  if (flag.kind === 'higher') return '↑';
  if (flag.kind === 'lower') return '↓';
  return (BAND_LABEL[flag.band ?? ''] ?? '').charAt(0);
}

/// Flagging is for one song at a time, and Energy is not a mood to flag.
export function flagControlVisible(entryCount: number, moodId: string): boolean {
  return entryCount <= 1 && moodId !== 'energy';
}
