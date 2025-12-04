# Build stage
FROM rust:1.75-slim-bookworm AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Create a new empty project
WORKDIR /app

# Copy manifests
COPY Cargo.toml Cargo.lock ./

# Create dummy source to cache dependencies
RUN mkdir src && \
    echo "fn main() {}" > src/main.rs && \
    echo "pub fn lib() {}" > src/lib.rs

# Build dependencies only (this layer will be cached)
RUN cargo build --release && \
    rm -rf src target/release/deps/daedra*

# Copy actual source code
COPY src ./src
COPY benches ./benches
COPY tests ./tests

# Build the actual application
RUN cargo build --release

# Runtime stage
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN useradd -m -u 1000 -s /bin/bash daedra

# Copy the binary from builder
COPY --from=builder /app/target/release/daedra /usr/local/bin/daedra

# Set ownership
RUN chown daedra:daedra /usr/local/bin/daedra

# Switch to non-root user
USER daedra

# Set working directory
WORKDIR /home/daedra

# Expose SSE port
EXPOSE 3000

# Health check
HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
    CMD daedra check || exit 1

# Default command (STDIO transport)
ENTRYPOINT ["daedra"]
CMD ["serve", "--transport", "stdio"]
