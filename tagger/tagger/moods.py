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
        "relaxing",
        "positive",
    ],
    "sorrowful": [
        "reflective, somber, sorrowful music",
        "sad, mournful, melancholic music",
        "wistful, grieving, nostalgic music",
    ],
    "wondrous": [
        "hopeful",
        "inspiring",
    ],
    "joyful": [
        "happy",
        "upbeat",
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
        "horror",
        "dark",
        "heavy",
    ],
    "heroic": [
        "heroic, triumphant, bold orchestral music",
        "a rousing victory anthem, brass and choir soaring",
        "adventurous, noble, uplifting epic music",
    ],
}

# Energy: positive minus negative mean similarity.
# Bare tag words: the model was trained on tag-like text. Never the bare
# word "fast", which the model reads against energy.
ENERGY_POS = [
    "energetic",
    "upbeat",
    "powerful",
]
ENERGY_NEG = [
    "slow",
    "relaxing",
    "calm",
]
