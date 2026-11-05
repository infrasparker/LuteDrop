"""Text prompts for the mood axes. Each axis is the difference between the
mean similarity to one pole's prompts and the other's.

The three axes follow the PAD mood model: Bright - Dark (pleasure),
Calm - Energetic (arousal) and Intimate - Grand (dominance)."""

BRIGHT = [
    "bright, warm, hopeful music",
    "light uplifting major-key music",
    "sunny cheerful music",
]
DARK = [
    "dark, grim, foreboding music",
    "brooding minor-key music",
    "bleak sinister music",
]
GRAND = [
    "grand, majestic music for a full orchestra",
    "epic, powerful, monumental orchestral music",
    "mighty brass and choir, commanding and imposing",
]
INTIMATE = [
    "intimate music for a few instruments",
    "modest, personal music played by a small group",
    "a solo instrument, close and unassuming",
]
ENERGETIC = [
    "energetic, lively, exciting music",
    "fast, agitated, intense music",
    "busy, driving, stimulating music",
]
CALM = [
    "calm, relaxed, tranquil music",
    "slow, sleepy, still music",
    "soothing, gentle, peaceful music",
]

# Deprecated: the old Airy - Heavy axis. Kept only so app versions up to
# v0.12, which read `light` from /score, keep working while the analyzer is
# deployed ahead of the app. Remove once those versions are retired.
LIGHT = [
    "airy, wispy, sparse ethereal music",
    "delicate floating atmospheric music",
    "thin translucent misty soundscape",
]
HEAVY = [
    "heavy, dense, pressing music",
    "thick powerful massive orchestral music",
    "weighty pounding full sound",
]

POLES = {
    "bright": BRIGHT,
    "dark": DARK,
    "grand": GRAND,
    "intimate": INTIMATE,
    "energetic": ENERGETIC,
    "calm": CALM,
    "light": LIGHT,
    "heavy": HEAVY,
}
