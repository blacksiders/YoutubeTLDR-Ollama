# syntax=docker/dockerfile:1.6

# -------- Builder --------
FROM rust:1-bookworm AS builder

WORKDIR /app

# System deps for native-tls (OpenSSL)
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev ca-certificates && \
    rm -rf /var/lib/apt/lists/*

# Cache deps
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY src ./src
COPY static ./static
COPY build.rs ./

# Build release binary
RUN cargo build --release

# -------- Runtime --------
FROM debian:bookworm-slim AS runtime

WORKDIR /app

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl && \
    rm -rf /var/lib/apt/lists/*

# Copy binary from builder
COPY --from=builder /app/target/release/YouTubeTLDR-Ollama /usr/local/bin/yt-tldr

# Default env for container (override as needed)
ENV TLDR_IP=0.0.0.0 \
    TLDR_PORT=8001 \
    OLLAMA_BASE_URL=http://host.docker.internal:11434 \
    OLLAMA_TIMEOUT_SECS=0

EXPOSE 8001

HEALTHCHECK --interval=30s --timeout=5s --retries=3 CMD curl -fsS http://127.0.0.1:8001/ || exit 1

ENTRYPOINT ["/usr/local/bin/yt-tldr"]
