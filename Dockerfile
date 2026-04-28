# syntax=docker/dockerfile:1

FROM rust:1.95.0-bookworm AS builder

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN cargo build --release --locked

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 10001 --shell /usr/sbin/nologin collector

COPY --from=builder /app/target/release/pingpongkong-k8s-collector /usr/local/bin/pingpongkong-k8s-collector

USER collector
EXPOSE 8080

ENTRYPOINT ["/usr/local/bin/pingpongkong-k8s-collector"]
