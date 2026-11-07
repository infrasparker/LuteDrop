"""Extra short phrases the tuner may try as prompt lines, beyond the words and
lines already in the user's moods. Seeded with wordings that worked in the
2026-11-06 hand-tuning sessions; they cover every default mood."""

PHRASES: list[str] = [
    # sorrowful
    "the saddest music", "a sad farewell", "music for a funeral", "remembrance",
    "memory, sad, lonely", "a lament", "grief and loss", "tender and heartbreaking",
    "a lullaby for the dead", "mourning", "bittersweet nostalgia", "quiet sorrow",
    # grim
    "horror, haunted house, terror, fear, scary", "an ominous presence", "creeping dread",
    "a dark ritual", "something evil awakens", "eerie and unsettling", "a cursed place",
    "a graveyard at night", "sinister whispers", "nightmare",
    # quirky
    "whimsical, playful, impish, curious, fairy", "a mischievous trickster", "silly and goofy",
    "a comic chase", "offbeat and funny", "a dancing puppet", "a circus", "bouncy and cheeky",
    # peaceful
    "a calm village", "a gentle morning", "a quiet campfire", "peaceful and relaxing",
    "a slow walk in the meadow", "lullaby", "soft and soothing", "a sleeping forest",
    # joyful
    "a joyful celebration", "a tavern dance", "a happy festival", "cheerful and bright",
    "a sunny day", "a merry jig", "laughter and cheer", "a wedding feast",
    # wondrous
    "a sacred choir", "angelic voices", "awe and wonder", "a glimpse of heaven",
    "a celestial vision", "magical and shimmering", "a hidden temple", "divine light",
    # mysterious
    "a mysterious secret", "an enigma", "curious and puzzling", "music box and chimes",
    "a hidden door", "sparse, ringing notes", "an unsolved riddle", "a strange discovery",
    "a candlelit library",
    # heroic
    "a hero's triumph", "a rousing march", "victory", "a noble quest",
    "epic orchestral adventure", "bold brass fanfare", "the knights ride out", "uplifting and heroic",
    # battle
    "a fierce battle", "war drums", "swords clashing", "a desperate fight",
    "an army charges", "combat with a monster", "relentless attack", "the final battle",
    # energy
    "slow and still", "high energy", "driving and intense", "calm and sparse",
    "fast and frantic", "soft and quiet", "loud and powerful", "gentle and slow",
]
