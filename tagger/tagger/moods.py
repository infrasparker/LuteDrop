"""Text prompts for the mood axes. Each axis is the difference between the
mean similarity to one pole's prompts and the other's."""

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

POLES = {"bright": BRIGHT, "dark": DARK, "light": LIGHT, "heavy": HEAVY}
