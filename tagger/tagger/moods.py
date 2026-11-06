"""Text prompts for the mood pools. Each pool is scored on its own as the
absolute mean similarity of a track with its prompts."""

# Mood pools: a flat list of hedged mood words, one per music track. Each
# pool is scored on its own as the absolute mean similarity with its prompts;
# the app z-scores all eight against a frozen calibration and suggests the
# best. Fixed vocabulary, in this order; the app knows these ids.
POOLS = {
    "peaceful": [
        "peaceful, tranquil, serene music",
        "calm relaxing music for resting",
        "gentle soothing pastoral music",
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
