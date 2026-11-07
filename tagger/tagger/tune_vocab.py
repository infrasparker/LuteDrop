"""The built-in vocabulary for mood auto-tune: words and short phrases the /tune search
may try (in Suggest mode) besides the app's word bank, the moods' own lines and PHRASES.
The app never sends it; its lines read "built-in vocabulary" in the review.

Curated from the 2026-11-05 feel-lab list (docs/superpowers/research/2026-11-05-feel-lab/
big_words.py): repeats and anything already in PHRASES dropped, and words too generic
to steer a mood ("odd", "close", "major", "theme", ...) or dated ("oriental") left out.
Hyphenated words read as spaced ("trance-like" -> "trance like").
"""
from .tune_phrases import PHRASES

EMOTION = """
sad sorrowful mournful grieving heartbroken melancholic melancholy wistful nostalgic longing yearning lonely
lonesome forlorn desolate bleak hopeless despairing tragic bittersweet tender tearful weeping somber solemn
regretful remorseful aching resigned defeated abandoned forgotten homesick pensive reflective contemplative
introspective meditative thoughtful dreamy wishful hopeful optimistic happy joyful cheerful merry jolly elated
euphoric ecstatic blissful delighted warm cozy comforting soothing calm peaceful serene tranquil relaxed
restful gentle soft mellow placid carefree playful whimsical mischievous impish cheeky silly goofy zany wacky
quirky eccentric weird strange bizarre absurd comical funny humorous witty sneaky curious intrigued puzzled
mysterious enigmatic cryptic secretive arcane occult mystical magical enchanted ethereal otherworldly uncanny
eerie creepy spooky haunting haunted ghostly sinister ominous menacing threatening foreboding dreadful fearful
afraid scared terrified horrified anxious nervous uneasy tense suspenseful paranoid unsettling disturbing
nightmarish macabre morbid grim gloomy dark brooding moody stormy angry furious wrathful aggressive hostile
violent brutal savage fierce ferocious relentless frantic chaotic urgent desperate panicked heroic brave
courageous valiant noble proud triumphant victorious glorious majestic regal grand epic legendary mythic 
awe-inspiring reverent sacred holy divine heavenly angelic celestial transcendent spiritual devout prayerful
hymnal uplifting inspiring rousing stirring determined resolute defiant bold confident powerful mighty strong
energetic lively upbeat bouncy spirited exuberant festive celebratory jubilant boisterous rowdy raucous
romantic loving affectionate intimate sensual passionate dramatic intense emotional moving poignant touching
heartwarming sentimental innocent childlike pure fragile delicate vulnerable shy lazy sleepy drowsy
""".split()

ATMOSPHERE = """
ambient atmospheric spacious vast expansive empty sparse minimal bare hollow cavernous echoing reverberant
misty foggy hazy cloudy shimmering glittering sparkling glowing radiant luminous bright sunny dim shadowy
murky gloomy dusky twilight nocturnal midnight moonlit starlit dawn sunrise sunset dusk winter wintry icy
frozen cold chilly frosty snowy autumn autumnal spring summer warm humid tropical rainy stormy windy breezy
oceanic underwater submerged deep subterranean underground ancient timeless primeval primal earthy organic
rustic pastoral rural natural wild untamed overgrown lush verdant forested barren arid dusty ruined decaying
crumbling abandoned desolate haunted cursed blighted sacred hallowed consecrated
""".split()

MUSIC = """
slow fast steady driving pulsing throbbing rhythmic syncopated groovy funky swinging lilting flowing drifting
floating swirling meandering building swelling rising soaring climactic sweeping cinematic orchestral
symphonic choral operatic chamber acoustic electronic synthetic ambient drone droning minimalist repetitive
hypnotic trance-like meditative sparse lush dense layered soft hushed whispered muted muffled distant faraway
intimate warm cold bright dark dissonant folk tribal medieval renaissance baroque classical romantic celtic
nordic eastern middle-eastern gothic industrial martial militaristic marching processional ceremonial lullaby
waltz jig reel shanty ballad dirge requiem elegy lament hymn chant anthem fanfare march melody melodic lyrical
wordless
""".split()

INSTRUMENTS = """
piano cello violin viola strings harp flute piccolo oboe clarinet bassoon horn french-horn trumpet trombone
tuba brass woodwinds organ choir voices bells chimes celesta glockenspiel music-box harpsichord lute guitar
acoustic-guitar mandolin fiddle bagpipes accordion hurdy-gurdy dulcimer zither koto sitar didgeridoo drums
war-drums taiko timpani percussion tambourine handpan kalimba marimba xylophone synth pads
""".split()

PLACES = """
tavern inn market village town city castle throne-room cathedral temple shrine monastery crypt tomb catacombs
dungeon prison sewer cave cavern mine mountain peak cliff valley meadow field farm orchard garden forest woods
jungle swamp marsh bog lake river waterfall sea ocean shore beach harbor ship desert tundra glacier volcano
ruins graveyard cemetery battlefield camp campfire road wilderness frontier library tower laboratory palace
ballroom feywild underdark heaven hell abyss
""".split()

INSTRUMENT_PHRASES = [
    "sad piano", "solo piano", "gentle piano", "slow piano", "lonely piano", "melancholic piano",
    "solo cello", "mournful cello", "sad strings", "slow strings", "sustained strings", "swelling strings",
    "pizzicato strings", "tremolo strings", "solo violin", "sad violin", "gentle harp", "soft flute",
    "lonely flute", "distant horn", "solemn brass", "heroic brass", "brass fanfare", "low brass",
    "church organ", "angelic choir", "dark choir", "wordless choir", "female vocals", "male choir",
    "tolling bells", "wind chimes", "music box melody", "plucked lute", "folk fiddle", "pounding drums",
    "war drums", "tribal drums", "low drones", "deep drone", "synth pads", "warm pads", "glassy textures",
]

SCENE_PHRASES = [
    "the saddest music", "a sad farewell", "dark sadness", "quiet sadness", "deep sorrow", "a funeral",
    "music for a funeral", "a funeral procession", "mourning the dead", "grief and loss", "a lament",
    "remembering someone who is gone", "a lost home", "after the battle", "the aftermath", "a fallen hero",
    "a lonely road", "a ruined village", "a dying world", "an empty house", "tears in the rain",
    "a broken heart", "a goodbye", "the end of an era", "a quiet memorial", "memories of home",
    "a calm village", "a quiet morning", "a gentle breeze", "resting by the fire", "a safe haven",
    "a cozy cottage", "a sleepy town", "a warm hearth", "a peaceful meadow", "a slow river",
    "a lazy afternoon", "a quiet garden", "home at last", "a gentle lullaby", "a tavern celebration",
    "a village festival", "a drinking song", "a merry dance", "a wedding feast", "a joyful reunion",
    "a lively jig", "a harvest festival", "a happy ending", "laughter in the tavern", "a fairy dance",
    "a mischievous imp", "a whimsical adventure", "a playful chase", "a goblin market", "a toy shop",
    "a comic misadventure", "a bumbling villain", "a strange carnival", "a funky groove", "a jazzy tune",
    "a silly march", "a hidden secret", "an ancient mystery", "a strange discovery", "a forgotten temple",
    "an arcane library", "a magical forest", "an enchanted grove", "a mysterious stranger", "a riddle",
    "secrets in the dark", "an otherworldly realm", "a dream world", "a mystic ritual", "a sacred place",
    "a holy relic", "a cathedral choir", "a celestial vision", "the heavens open", "an ancient god",
    "a breathtaking vista", "the top of the world", "the northern lights", "a divine presence",
    "a moment of awe", "a vast landscape", "a haunted house", "a horror movie", "something is watching",
    "a creeping horror", "a monster approaches", "a nightmare", "an evil ritual", "a cursed crypt",
    "the undead rise", "a dark forest at night", "a plague", "a torture chamber", "an abandoned asylum",
    "a demonic presence", "a hero rises", "a triumphant return", "a royal procession",
    "the call to adventure", "a noble quest", "a battle cry", "a fierce battle", "a boss fight",
    "a desperate struggle", "an army marches", "a duel", "a chase", "an ambush", "the final battle",
    "a siege", "a dragon attacks",
]



def _vocab() -> list[str]:
    taken = {p.lower() for p in PHRASES}
    seen: dict[str, None] = {}
    for w in EMOTION + ATMOSPHERE + MUSIC + INSTRUMENTS + PLACES + INSTRUMENT_PHRASES + SCENE_PHRASES:
        w = w.replace("-", " ").lower()
        if w not in taken:
            seen.setdefault(w)
    return list(seen)


VOCAB: list[str] = _vocab()
