"""Text prompts for the mood pools. Each pool is scored on its own as the
absolute mean similarity of a track with its prompts."""

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
