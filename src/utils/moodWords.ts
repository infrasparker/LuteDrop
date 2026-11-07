// Mood words shared by the Moods panel (MoodSettingsDialog.vue) and the Tune
// dialog (MoodTuneDialog.vue).

/// Tag words the analyzer's text model knows well (MTG-Jamendo moods and
/// themes). The Moods panel offers them as a word bank; Tune sends them as
/// part of its candidate bank.
export const WORD_BANK = [
  'action', 'adventure', 'ambiental', 'calm', 'dark', 'deep', 'dramatic', 'dream', 'emotional',
  'energetic', 'epic', 'fast', 'fun', 'funny', 'groovy', 'happy', 'heavy', 'hopeful', 'horror',
  'inspiring', 'meditative', 'melancholic', 'mellow', 'melodic', 'motivational', 'nature', 'party',
  'positive', 'powerful', 'relaxing', 'sad', 'slow', 'soft', 'soundscape', 'space', 'travel',
  'upbeat', 'uplifting',
];

/// A fresh internal id for a new mood: "m" and 16 random lowercase letters
/// (about 75 bits, so collisions are not a concern). It stays within the
/// a-z, at most 32 characters rule that the backend and the analyzer check,
/// so random ids need no change on either side; the shipped moods keep their
/// readable ids (peaceful, sorrowful, ...).
export function newMoodId(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  return 'm' + Array.from(bytes, (b) => String.fromCharCode(97 + (b % 26))).join('');
}
