"""Bounded analyzer-container experiment: embed supplied text, no service writes."""
import json,sys
from pathlib import Path
import numpy as np
sys.path.insert(0,'/app')
from tagger.embed import MuqEmbedder
texts=json.loads(Path(sys.argv[1]).read_text())
model=MuqEmbedder();model.load()
vectors=model.embed_text(texts)
np.savez_compressed(sys.argv[2],texts=np.array(texts),vectors=vectors)
print('Exported',len(texts),'prompt vectors',flush=True)
