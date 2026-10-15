FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends bash coreutils curl git jq python3 python3-pip ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /workspace
CMD ["sleep", "infinity"]
