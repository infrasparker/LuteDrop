"""Text prompts for the mood profile: nine moods plus Energy.

Each mood is scored on its own as the absolute mean similarity of a track with
its prompts. Energy is the one two-sided score: the mean similarity with the
positive prompts minus the mean similarity with the negative ones. The app
calibrates all ten raw scores into 0..1. Fixed vocabulary, in this order; the
app knows these ids.
"""

MOODS = {
    "peaceful": [
        "peaceful, tranquil, serene music",
        "calm relaxing music for resting",
        "gentle soothing pastoral music",
    ],
    "sorrowful": [
        "reflective, somber, sorrowful music",
        "sad, mournful, melancholic music",
        "wistful, grieving, nostalgic music",
    ],
    "wondrous": [
        "music full of wonder and awe",
        "sacred, divine, celestial music, a choir in a cathedral",
        "a breathtaking vista, hopeful and vast, slow and soaring",
    ],
    "cheerful": [
        "cheerful, merry, jaunty music",
        "a lively folk dance, upbeat and happy",
        "bright, fun, festive tune",
    ],
    "quirky": [
        "quirky, weird, offbeat music",
        "groovy, funky, eccentric and strange",
        "campy, oddball, mischievous tune",
    ],
    "mysterious": [
        "mysterious, unsettling, eerie music",
        "strange, uncanny, otherworldly sounds",
        "quiet suspense and looming dread, something is about to happen",
    ],
    "grim": [
        "dark, tense, mid-tempo combat music",
        "grim, determined, pressing fight music, no fanfare",
        "a hard fight in the dark, drums and low strings",
    ],
    "heroic": [
        "heroic, triumphant, bold orchestral music",
        "a rousing victory anthem, brass and choir soaring",
        "adventurous, noble, uplifting epic music",
    ],
    "menacing": [
        "dark, brutal, menacing battle music",
        "aggressive pounding drums, dissonant and threatening",
        "a monstrous boss fight, violent and grim",
    ],
}

# Energy: positive minus negative mean similarity.
ENERGY_POS = [
    "energetic, lively, exciting music",
    "fast, agitated, intense music",
    "busy, driving, stimulating music",
]
ENERGY_NEG = [
    "calm, relaxed, tranquil music",
    "slow, sleepy, still music",
    "soothing, gentle, peaceful music",
]
