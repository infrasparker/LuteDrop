// Pure helpers for flagging a mood bar that looks wrong (see MoodProfile.vue).
import type { MoodFlag } from '@generated/index';

/// The bands a flag can name, in the order the menu lists them.
export const FLAG_BANDS = [
  { band: 'definitely', label: 'Definitely', menuLabel: 'Definitely' },
  { band: 'understandable', label: 'Understandable', menuLabel: 'Understandable' },
  { band: 'stretch', label: 'Stretch', menuLabel: 'Stretch' },
  { band: 'no', label: 'No', menuLabel: 'No' },
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
    text = flag.bound === 'at_most' && ['definitely','understandable'].includes(flag.band ?? '') ? `${band} or less` : flag.bound === 'at_least' && ['stretch','no'].includes(flag.band ?? '') ? `${band} or more` : band;
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
  if (level !== null) return `Hand-set: ${level.toFixed(2)}`;
  return 'flag removed';
}

/// The short mark beside a flagged bar's icon: the band's initial (D / U /
/// S / N) or an arrow for higher / lower.
export function flagMark(flag: Pick<MoodFlag, 'kind' | 'band'>): string {
  if (flag.kind === 'higher') return '↑';
  if (flag.kind === 'lower') return '↓';
  return ({definitely:'D', understandable:'U', stretch:'S', no:'N'})[flag.band ?? 'no'];
}

/// Flagging is for one song at a time, and Energy is not a mood to flag.
export function flagControlVisible(entryCount: number, moodId: string): boolean {
  return entryCount <= 1 && moodId !== 'energy';
}

/** Strength targets, not thresholds. Stored band IDs remain compatible. */
export function ratingTarget(flag: Pick<MoodFlag, 'kind' | 'band' | 'barAtFlag'> | null, level: number | null, auto: number | null = null): number | null {
  if (!flag) return level;
  if (flag.kind === 'band') return ({no: .1, stretch: .35, understandable: .65, definitely: .9})[flag.band ?? 'no'];
  const anchor = flag.barAtFlag ?? auto;
  return anchor === null ? null : Math.max(0, Math.min(1, anchor + (flag.kind === 'higher' ? .1 : -.1)));
}

/** Version 4: judgments express membership and conviction, never a score target. */
export function feedbackPreference(flag: Pick<MoodFlag,'kind'|'band'|'bound'|'barAtFlag'> | null, level: number | null, auto: number | null = null) {
  if (!flag) return level === null ? null : {direction:0, confidence:1, pivot:.5, numeric:level, min:null, max:null};
  const confidence = flag.band === 'definitely' || flag.band === 'no' ? 4 : 1;
  if (flag.kind !== 'band') return {direction:flag.kind === 'higher' ? 1 : -1, confidence:1, pivot:flag.barAtFlag ?? auto ?? .5, numeric:null, min:null, max:null};
  const positive = flag.band === 'definitely' || flag.band === 'understandable';
  if (positive && flag.bound === 'at_most') return {direction:-1, confidence, pivot:flag.band === 'understandable' ? .9 : 1, numeric:null, min:null, max:flag.band === 'understandable' ? .9 : 1};
  if (!positive && flag.bound === 'at_least') return {direction:1, confidence, pivot:flag.band === 'stretch' ? .5 : 0, numeric:null, min:flag.band === 'stretch' ? .5 : 0, max:null};
  return {direction:positive ? 1 : -1, confidence, pivot:.5, numeric:null, min:null, max:null};
}
export function feedbackDisagreement(value: number, preference: ReturnType<typeof feedbackPreference>): number {
  if (!preference) return 0;
  if (preference.numeric !== null) return (value-preference.numeric)**2;
  if (!preference.direction) return Math.max(0,(preference.min ?? 0)-value,value-(preference.max ?? 1))**2;
  const latent = Math.tan(Math.PI*(Math.max(1e-12,Math.min(1-1e-12,value))-.5));
  const pivot = Math.max(.00001,Math.min(.99999,preference.pivot));
  const p = .5+Math.atan(latent-Math.tan(Math.PI*(pivot-.5)))/Math.PI;
  return -Math.log(Math.max(1e-12,preference.direction > 0 ? p : 1-p));
}
export function feedbackDescription(preference: ReturnType<typeof feedbackPreference>): string {
  if (!preference) return 'No feedback';
  if (preference.numeric !== null) return `Level set by you: ${preference.numeric.toFixed(2)}`;
  if (preference.min !== null || preference.max !== null) return preference.max !== null ? `At most ${preference.max.toFixed(2)}` : `At least ${(preference.min ?? 0).toFixed(2)}`;
  if (preference.pivot !== .5) return `${preference.direction>0 ? 'Higher' : 'Lower'} than the original ${preference.pivot.toFixed(3)}`;
  return `${preference.confidence>1 ? 'Strong ' : ''}${preference.direction>0 ? 'yes · higher scores agree more' : 'no · lower scores agree more'}`;
}
