# The agent computer for Claude turns when "Run Claude in the computer" is on (Linux, slice 1;
# src/sandbox/agent_vm). kwnt-executor plus socat and one pinned Claude Code. The container runs
# with no network; socat forwards 127.0.0.1:8080 to kwnt's broker socket at /run/kwnt.
FROM kwnt-executor:v1
ARG CLAUDE_VERSION
RUN apt-get update && apt-get install -y --no-install-recommends socat util-linux \
    && rm -rf /var/lib/apt/lists/*
# Anthropic's installer checks the download against its published checksum; the binary is then
# moved out of the throwaway home so every user id the container runs as can start it
RUN set -eu; test -n "$CLAUDE_VERSION"; export HOME=/tmp/claude-install; mkdir -p "$HOME"; \
    curl -fsSL https://claude.ai/install.sh | bash -s "$CLAUDE_VERSION"; \
    install -m 0755 "$(readlink -f "$HOME/.local/bin/claude")" /usr/local/bin/claude; \
    rm -rf "$HOME"; /usr/local/bin/claude --version | grep -F "$CLAUDE_VERSION"
COPY agent-entry.sh /usr/local/bin/kwnt-agent-entry
RUN chmod 0755 /usr/local/bin/kwnt-agent-entry
CMD ["/usr/local/bin/kwnt-agent-entry"]
