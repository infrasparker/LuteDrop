"""Default text prompts for the mood profile: eight moods plus Energy.

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
        "sad, sorrowful, mournful, melancholic, somber, wistful, grieving, nostalgic, reflective music",
    ],
    "wondrous": [
        "hopeful, inspiring",
    ],
    "joyful": [
        "happy, upbeat",
    ],
    "quirky": [
        "quirky, weird, offbeat, groovy, funky, eccentric, strange, campy, oddball, mischievous music",
    ],
    "mysterious": [
        "mysterious, unsettling, eerie, strange, uncanny, otherworldly, suspenseful, looming, dread music",
    ],
    "grim": [
        "horror, dark, heavy",
    ],
    "heroic": [
        "heroic, triumphant, bold, rousing, victorious, adventurous, noble, uplifting, epic orchestral music",
    ],
}

# Energy: positive minus negative mean similarity.
# One cluster of tag words per side: the model was trained on tag-like text.
# Never the bare word "fast", which the model reads against energy.
ENERGY_POS = [
    "energetic, upbeat, powerful",
]
ENERGY_NEG = [
    "slow, relaxing, calm",
]
