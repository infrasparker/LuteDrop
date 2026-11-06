"""Default text prompts for the mood profile: nine moods plus Energy.

Each mood is scored on its own as the absolute mean similarity of a track with
its prompts. Energy is the one two-sided score: the mean similarity with the
positive prompts minus the mean similarity with the negative ones. The app
calibrates the raw scores into 0..1.

These are defaults only. The app keeps its own, editable vocabulary and sends
it with each /score request (`mood_prompts`, `energy_prompts`); these are
used when a request leaves them out.
"""

MOODS = {
    "peaceful": [
        "relaxing, positive",
    ],
    "sorrowful": [
        "sad, melancholic, somber, funeral, lament, tragic, heartbroken music",
    ],
    "wondrous": [
        "angelic, divine, sacred, celestial, awe, wonder, inspiring",
    ],
    "joyful": [
        "joyful, happy, cheerful, merry, bright, glad, festive, sunny, drinking song",
    ],
    "quirky": [
        "quirky, offbeat, groovy, funky, oddball, mischievous, funny, silly, goofy, zany music",
    ],
    "mysterious": [
        "mysterious, enigmatic, puzzling, curious, intrigue, chimes, bells, celesta, music box, sparse isolated notes, high ringing",
    ],
    "grim": [
        "ominous, dread, fearful, scary, horror, terror, sinister, eldritch, unnatural music",
    ],
    "heroic": [
        "heroic, triumphant, bold, rousing, victorious, adventurous, noble, uplifting, epic orchestral music",
    ],
    "battle": [
        "combat, battle, warlike, fierce, clash, war drums, fantasy battle music",
    ],
}

# Energy: positive minus negative mean similarity.
# One cluster of tag words per side: the model was trained on tag-like text.
# Never the bare word "fast", which the model reads against energy.
ENERGY_POS = [
    "energetic, exciting, intense, upbeat, powerful",
]
ENERGY_NEG = [
    "slow, relaxing, calm",
]
