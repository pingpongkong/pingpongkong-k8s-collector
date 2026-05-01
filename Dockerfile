# syntax=docker/dockerfile:1

FROM rust:1.95.0-bookworm AS builder

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN cargo build --release --locked

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates git \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 10001 --shell /usr/sbin/nologin collector

COPY --from=builder /app/target/release/pingpongkong-k8s-collector /usr/local/bin/pingpongkong-k8s-collector

ENV RUST_BACKTRACE=1
ENV COLLECTOR_API_PORT=8081

USER collector
EXPOSE 8081

ENTRYPOINT ["/usr/local/bin/pingpongkong-k8s-collector"]
