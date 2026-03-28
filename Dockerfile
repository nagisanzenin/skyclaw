# ============================================================================
# TEMM1E — Cloud-Native AI Agent Runtime
# Multi-stage Docker build with all features: Telegram, Discord, Browser,
# MCP, Codex OAuth, TUI, Desktop Control (Tem Gaze), and Prowl web-native browsing.
# ============================================================================

# ---- Builder stage ----
FROM rust:1.88-bookworm AS builder

ARG GIT_HASH=unknown
ARG BUILD_DATE=unknown
ARG FEATURES=telegram,discord,browser,mcp,codex-oauth,tui,desktop-control


# Build dependencies for desktop-control (xcap → wayland/xcb)
RUN apt-get update && apt-get install -y --no-install-recommends \
        libwayland-dev libxcb1-dev libxcb-randr0-dev libxcb-shm0-dev \
        libxkbcommon-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# 1) Copy manifests + build.rs for dependency caching.
#    Create stub main/lib files so cargo can resolve the dep graph and
#    cache compiled dependencies before copying real source.
COPY Cargo.toml Cargo.lock build.rs ./
COPY crates/ crates/
RUN mkdir -p src && echo 'fn main() {}' > src/main.rs

# 2) Build dependencies only (cached layer — survives source changes).
ENV GIT_HASH=${GIT_HASH}
ENV BUILD_DATE=${BUILD_DATE}
RUN cargo build --release --features "${FEATURES}" 2>/dev/null || true

# 3) Copy real source and build the binary.
COPY src/ src/
RUN touch src/main.rs && cargo build --release --features "${FEATURES}"

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
    && rm -rf /var/lib/apt/lists/*

# Chromium path for chromiumoxide (Prowl browser engine)
ENV CHROME_PATH=/usr/bin/chromium

# Default config directory (mount a volume here for persistence)
ENV TEMM1E_HOME=/data
RUN mkdir -p /data

WORKDIR /app

COPY --from=builder /app/target/release/temm1e ./temm1e

# Gateway port
EXPOSE 8080

# Health check — gateway /health endpoint (10s interval, 3 retries)
HEALTHCHECK --interval=10s --timeout=5s --start-period=15s --retries=3 \
    CMD curl -sf http://localhost:8080/health || exit 1

# Use tini as PID 1 for proper signal handling.
# Default command: start the gateway. Override with "chat" or "tui" for
# interactive modes: docker run -it temm1e chat
ENTRYPOINT ["tini", "--", "./temm1e"]
CMD ["start"]
