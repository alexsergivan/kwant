FROM debian:bookworm-slim

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
    bash \
    coreutils \
    curl \
    git \
    jq \
    python3 \
    python3-pip \
    ca-certificates \
    chromium \
    socat \
    fonts-liberation \
    fonts-noto-color-emoji \
    && rm -rf /var/lib/apt/lists/*

# Configure Chromium managed policies for password manager and autofill
RUN mkdir -p /etc/chromium/policies/managed && \
    echo '{"PasswordManagerEnabled": true, "AutofillAddressEnabled": true, "AutofillCreditCardEnabled": true, "AutoFillEnabled": true, "CredentialsEnableService": true}' > /etc/chromium/policies/managed/policies.json

WORKDIR /workspace

EXPOSE 9222

COPY docker/entrypoint.sh /entrypoint.sh
RUN chmod 755 /entrypoint.sh

ENTRYPOINT ["/entrypoint.sh"]
