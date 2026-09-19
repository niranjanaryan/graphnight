# Multi-stage build for graphnight-server + graphnight CLI
FROM rust:bookworm AS builder

WORKDIR /app

# Cache dependency builds
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

RUN cargo build --release -p graphnight-server -p graphnight-cli \
    && strip target/release/graphnight-server target/release/graphnight

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home --uid 10001 graphnight

WORKDIR /app

COPY --from=builder /app/target/release/graphnight-server /usr/local/bin/graphnight-server
COPY --from=builder /app/target/release/graphnight /usr/local/bin/graphnight
COPY examples /app/examples

RUN mkdir -p /data && chown -R graphnight:graphnight /app /data

USER graphnight

ENV GRAPHNIGHT_STORAGE_PATH=/data
VOLUME ["/data"]
EXPOSE 8080

ENTRYPOINT ["graphnight-server"]
CMD ["--host", "0.0.0.0", "--port", "8080", "--storage-path", "/data", "--config", "/app/examples/graphnight.toml"]
