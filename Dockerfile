# ============================================================================
# TEMM1E — Cloud-Native AI Agent Runtime
# Multi-stage Docker build with all features: Telegram, Discord, Browser,
# MCP, Codex OAuth, TUI, Desktop Control (Tem Gaze), and Prowl web-native browsing.
# ============================================================================

# ---- Builder stage ----
FROM rust:1.91.1-bookworm AS builder

ARG GIT_HASH=unknown
ARG BUILD_DATE=unknown
ARG FEATURES=telegram,discord,browser,mcp,codex-oauth,tui,desktop-control


# Build dependencies for desktop-control (xcap + enigo)
RUN apt-get update && apt-get install -y --no-install-recommends \
        libwayland-dev libxcb1-dev libxcb-randr0-dev libxcb-shm0-dev \
        libxkbcommon-dev libpipewire-0.3-dev libspa-0.2-dev libclang-dev \
        libegl1-mesa-dev libgbm-dev libdrm-dev libxdo-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Use the locked dependency graph and the workspace minimum Rust version.
# Build once with real sources: a best-effort stub build hid dependency errors
# and stored a second set of artifacts in the image layer cache.
COPY Cargo.toml Cargo.lock build.rs ./
COPY crates/ crates/
COPY src/ src/
ENV GIT_HASH=${GIT_HASH}
ENV BUILD_DATE=${BUILD_DATE}
RUN cargo build --locked --release --features "${FEATURES}"

# ---- Runtime stage ----
FROM debian:bookworm-slim

# OCI image labels
LABEL org.opencontainers.image.title="TEMM1E" \
      org.opencontainers.image.description="Cloud-native Rust AI agent runtime" \
      org.opencontainers.image.source="https://github.com/temm1e-labs/temm1e" \
      org.opencontainers.image.licenses="MIT"

# Runtime dependencies:
#   ca-certificates   — TLS for API calls (Anthropic, OpenAI, Gemini, etc.)
#   chromium          — headless browser for Prowl web-native browsing
#   tini              — proper PID 1 signal forwarding (SIGTERM → graceful shutdown)
#   curl              — health check probe
#   tzdata            — timezone support for cron jobs and timestamps
#   libxcb1, libxkbcommon0 — runtime libs for xcap screen capture (desktop-control)
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
        chromium \
        tini \
        curl \
        tzdata \
        libxcb1 \
        libxcb-randr0 \
        libxcb-shm0 \
        libxkbcommon0 \
        libxdo3 \
    && rm -rf /var/lib/apt/lists/*

# Chromium path for chromiumoxide (Prowl browser engine)
ENV CHROME_PATH=/usr/bin/chromium

# Default config directory (mount a volume here for persistence)
ENV TEMM1E_DATA_DIR=/var/lib/temm1e
RUN mkdir -p /var/lib/temm1e

WORKDIR /app

COPY --from=builder /app/target/release/temm1e ./temm1e

# A successful link in the builder does not prove runtime libraries exist.
RUN ldd ./temm1e && ./temm1e --version

# Gateway port
EXPOSE 8080

# Health check — gateway /health endpoint (10s interval, 3 retries)
HEALTHCHECK --interval=10s --timeout=5s --start-period=15s --retries=3 \
    CMD curl -sf http://localhost:8080/health || exit 1

# Use tini as PID 1 for proper signal handling.
# Default command: start the gateway. Override with "chat" or "tui" for
# interactive modes: docker run -it temm1e chat
ENTRYPOINT ["tini", "--", "./temm1e"]
CMD ["start", "--host", "0.0.0.0"]
