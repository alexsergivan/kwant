# Laya routing sidecar with model weights baked in, so it starts offline.
FROM python:3.12-slim

ARG LAYA_VERSION=0.4.8
ARG LAYA_MODELS=english

ENV PIP_NO_CACHE_DIR=1 \
    PYTHONUNBUFFERED=1 \
    HF_HOME=/opt/hf \
    LAYA_HOST=0.0.0.0 \
    LAYA_PORT=8000 \
    LAYA_MODELS=${LAYA_MODELS} \
    LAYA_DEFAULT_MODEL=english

# CPU-only PyTorch keeps the image a fraction of the CUDA build's size.
RUN pip install --index-url https://download.pytorch.org/whl/cpu torch \
 && pip install "laya[serve]==${LAYA_VERSION}"

# Download and load the checkpoints once at build time.
RUN python -c "from laya.serve import build_router; build_router()"

ENV HF_HUB_OFFLINE=1
EXPOSE 8000
HEALTHCHECK --interval=10s --timeout=3s --start-period=120s --retries=3 \
  CMD python -c "import urllib.request; urllib.request.urlopen('http://127.0.0.1:8000/health')"
CMD ["laya-serve"]
