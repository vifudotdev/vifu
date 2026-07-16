# syntax=docker/dockerfile:1

ARG RUST_VERSION=1.90
FROM rust:${RUST_VERSION}-bookworm AS build

WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates/vifu/Cargo.toml crates/vifu/Cargo.toml
COPY crates/vifu-server/Cargo.toml crates/vifu-server/Cargo.toml
COPY crates/vifu-server/build.rs crates/vifu-server/build.rs
COPY crates/vifu-server/migrations crates/vifu-server/migrations
COPY crates ./crates
RUN cargo build --release --locked --workspace

FROM debian:bookworm-slim AS runtime-base

ARG UID=10001
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
RUN useradd \
    --create-home \
    --home-dir /home/vifu \
    --shell /usr/sbin/nologin \
    --uid "${UID}" \
    vifu
RUN mkdir -p /home/vifu/.vifu \
    && chown -R vifu:vifu /home/vifu/.vifu

USER vifu
WORKDIR /home/vifu

FROM runtime-base AS vifu-server
COPY --from=build /src/target/release/vifu-server /usr/local/bin/vifu-server
EXPOSE 6790
ENTRYPOINT ["vifu-server"]

FROM runtime-base AS vifu
COPY --from=build /src/target/release/vifu /usr/local/bin/vifu
ENTRYPOINT ["vifu"]
