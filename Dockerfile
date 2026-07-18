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
RUN --mount=type=cache,id=vifu-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=vifu-cargo-git,target=/usr/local/cargo/git \
    --mount=type=cache,id=vifu-cargo-target,target=/src/target \
    cargo build --release --locked -p vifu -p vifu-server \
    && mkdir -p /out \
    && cp /src/target/release/vifu /out/vifu \
    && cp /src/target/release/vifu-server /out/vifu-server

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
COPY --from=build /out/vifu-server /usr/local/bin/vifu-server
EXPOSE 6790
ENTRYPOINT ["vifu-server"]

FROM runtime-base AS vifu
COPY --from=build /out/vifu /usr/local/bin/vifu
ENTRYPOINT ["vifu"]
