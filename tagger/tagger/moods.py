"""Text prompts for the mood axes. Each axis is the difference between the
mean similarity to one pole's prompts and the other's.

The three axes follow the PAD mood model: Bright - Dark (pleasure),
Calm - Energetic (arousal) and Intimate - Grand (dominance).

The mood pools (POOLS) are not axes: each is scored on its own as the
absolute mean similarity with its prompts."""

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

POLES = {
    "bright": BRIGHT,
    "dark": DARK,
    "grand": GRAND,
    "intimate": INTIMATE,
    "energetic": ENERGETIC,
    "calm": CALM,
}

# Mood pools: a flat list of hedged mood words, one per music track. Each
# pool is scored on its own as the absolute mean similarity with its prompts;
# the app z-scores the six against a frozen calibration and suggests the best.
# The seventh pool, tension, has no prompts: it is taught only, never
# suggested. Fixed vocabulary; the app knows these ids.
POOLS = {
    "peaceful": [
        "peaceful, tranquil, serene music",
        "calm relaxing music for resting",
        "gentle soothing pastoral music",
    ],
    "lighthearted": [
        "lighthearted, fun, cheerful music",
        "playful, jaunty, merry tune",
        "goofy, quirky, whimsical music",
    ],
    "somber": [
        "reflective, somber, sorrowful music",
        "sad, mournful, melancholic music",
        "wistful, grieving, nostalgic music",
    ],
    "awe": [
        "music full of wonder and awe",
        "sacred, divine, celestial music, a choir in a cathedral",
        "a breathtaking vista, hopeful and vast, slow and soaring",
    ],
    "mysterious": [
        "mysterious, unsettling, eerie music",
        "strange, uncanny, otherworldly sounds",
        "quiet suspense and looming dread, something is about to happen",
    ],
    "epic": [
        "epic, intense, heroic music",
        "powerful triumphant orchestral battle music",
        "driving, aggressive, dramatic action music",
    ],
}
