"""Transient analyzer-container experiment; export text vectors, no service changes."""
import json
import sys
from pathlib import Path
import numpy as np

sys.path.insert(0, '/app')
from tagger.embed import MuqEmbedder
from tagger.tune_vocab import VOCAB
from tagger.tune_phrases import PHRASES

snapshot = json.loads(Path(sys.argv[1]).read_text())
families = json.loads(Path(sys.argv[2]).read_text())
texts = list(dict.fromkeys(VOCAB + PHRASES + [line for m in snapshot['moods'] for line in m['prompts']] + [line for f in families for line in f['sets']]))
model = MuqEmbedder()
model.load()
vectors = []
for off in range(0, len(texts), 32):
    vectors.extend(model.embed_text(texts[off:off+32]))
    print('TEXTS', min(off+32, len(texts)), '/', len(texts), flush=True)
np.savez_compressed(sys.argv[3], texts=np.array(texts), vectors=np.array(vectors))
