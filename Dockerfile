# Runtime-only image (binary pre-built on host)
FROM debian:bookworm-slim

RUN apt-get update && \
    DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        ca-certificates curl && \
    rm -rf /var/lib/apt/lists/* && \
    useradd --create-home --uid 1000 --shell /bin/bash daedra

COPY target/release/daedra /usr/local/bin/daedra
RUN chmod +x /usr/local/bin/daedra

WORKDIR /app
USER daedra

EXPOSE 3400

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -fsS http://127.0.0.1:3400/health || exit 1

ENTRYPOINT ["daedra"]
CMD ["serve", "--transport", "sse", "--port", "3400", "--host", "127.0.0.1"]
