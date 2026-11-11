"""Bounded wording proposals for the same signed scorer used by Preview.

There is no positive-only surrogate. These are alternatives, not automatic
edits or claims of improved subjective accuracy.
"""
import re


def key(lines):
    return tuple(sorted(line.strip().casefold() for line in lines if line.strip()))


def candidates(current, extra, bank, mode, previous=(), excluded=()):
    base = list(dict.fromkeys(line.strip() for line in (extra or current) if line.strip()))
    seen = {key(base), *(key(lines) for lines in previous)}
    output = []
    def add(lines):
        lines = list(dict.fromkeys(lines))
        if not lines or len(lines) > 16 or any(len(s) > 300 for s in lines) or not any(not s.startswith('- ') for s in lines): return
        signature = key(lines)
        if signature in seen: return
        seen.add(signature); output.append(lines)
    parts = []
    for i, line in enumerate(base):
        negative = line.startswith('- '); text = line[2:] if negative else line.removeprefix('+ ')
        add(base[:i]+base[i+1:])
        add(base[:i]+[text if negative else '- '+text]+base[i+1:])
        words = [word.strip() for word in text.split(',') if word.strip()]
        if len(words) > 1: add(base[:i]+[('- ' if negative else '')+w for w in words]+base[i+1:])
        parts.extend(words)
    for i, first in enumerate(base):
        for j, second in enumerate(base[i+1:], i+1):
            if first.startswith('- ') != second.startswith('- '): continue
            prefix = '- ' if first.startswith('- ') else ''
            phrase = ', '.join([first.removeprefix('- '), second.removeprefix('- ')])
            if len(phrase)+len(prefix) <= 300:
                add([line for k,line in enumerate(base) if k not in (i,j)]+[prefix+phrase])
    pool = list(dict.fromkeys(parts + (list(bank) if mode == 'suggest' else [])))
    for word in pool[:48]:
        word = word.strip().removeprefix('- ').strip()
        if not word or len(word) > 298: continue
        if any(re.search(r'\b'+re.escape(x)+r'\b', word, re.I) for x in excluded): continue
        # A standalone alternative replaces that term inside an existing comma
        # group. Do not propose duplicated/contradictory copies of one word as
        # an opaque way to give it extra influence.
        remainder = []
        for line in base:
            negative = line.startswith('- ')
            text = line[2:] if negative else line.removeprefix('+ ')
            pieces = [p.strip() for p in text.split(',') if p.strip().casefold() != word.casefold()]
            if pieces: remainder.append(('- ' if negative else '')+', '.join(pieces))
        add(remainder+[word]); add(remainder+['- '+word])
        if len(output) >= 120: break
    return output[:120]
