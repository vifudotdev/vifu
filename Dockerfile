# syntax=docker/dockerfile:1

ARG RUST_VERSION=1.95
FROM rust:${RUST_VERSION}-bookworm AS build

RUN apt-get update \
    && apt-get install -y --no-install-recommends clang cmake libclang-dev \
    && rm -rf /var/lib/apt/lists/*

# Build whisper.cpp for the portable container CPU baseline, not the build host.
ENV GGML_NATIVE=OFF

WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates/vifu-gateway/Cargo.toml crates/vifu-gateway/Cargo.toml
COPY crates/vifu-runtime/Cargo.toml crates/vifu-runtime/Cargo.toml
COPY crates/vifu/Cargo.toml crates/vifu/Cargo.toml
COPY crates/vifu-server/Cargo.toml crates/vifu-server/Cargo.toml
COPY crates/vifu-server/build.rs crates/vifu-server/build.rs
COPY crates/vifu-server/migrations crates/vifu-server/migrations
COPY crates ./crates
RUN --mount=type=cache,id=vifu-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=vifu-cargo-git,target=/usr/local/cargo/git \
    --mount=type=cache,id=vifu-cargo-target,target=/src/target \
    cargo build --release --locked -p vifu \
    && mkdir -p /out \
    && cp /src/target/release/vifu /out/vifu

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

FROM runtime-base AS runtime
COPY --from=build /out/vifu /usr/local/bin/vifu
EXPOSE 6790
ENTRYPOINT ["vifu"]
