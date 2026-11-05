FROM python:3.11-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends libsndfile1 ffmpeg \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# torch, torchvision and torchaudio must all come from the same CUDA index.
RUN pip install --no-cache-dir torch torchvision torchaudio \
    --index-url https://download.pytorch.org/whl/cu126

COPY requirements.txt requirements-model.txt ./
RUN pip install --no-cache-dir -r requirements-model.txt -r requirements.txt

COPY tagger ./tagger

ENV TAGGER_CACHE_DIR=/cache
EXPOSE 8765
CMD ["uvicorn", "tagger.app:factory", "--factory", "--host", "0.0.0.0", "--port", "8765"]
