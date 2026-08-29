# Build stage: compile daedra inside the image. No pre-built host binary is
# needed — `docker build` works from a clean checkout.
FROM rust:1.98-slim AS builder
WORKDIR /build
RUN apt-get update && \
    DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        pkg-config && \
    rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY tests ./tests
# Build with a stub main first is unnecessary: the crate is small, so a full
# build stays under the layer-cache value. Build the binary directly.
RUN cargo build --release

# Runtime stage: minimal image with only the binary and its runtime needs.
FROM debian:bookworm-slim

RUN apt-get update && \
    DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        ca-certificates curl && \
    rm -rf /var/lib/apt/lists/* && \
    useradd --create-home --uid 1000 --shell /bin/bash daedra

COPY --from=builder /build/target/release/daedra /usr/local/bin/daedra
RUN chmod +x /usr/local/bin/daedra

WORKDIR /app
USER daedra

EXPOSE 3400

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -fsS http://127.0.0.1:3400/health || exit 1

ENTRYPOINT ["daedra"]
