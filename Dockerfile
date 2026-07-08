# syntax=docker/dockerfile:1

ARG RUST_VERSION=1.90
FROM rust:${RUST_VERSION}-bookworm AS build

WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked

FROM debian:bookworm-slim AS runtime

ARG UID=10001
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

RUN useradd \
    --create-home \
    --home-dir /home/vifu \
    --shell /usr/sbin/nologin \
    --uid "${UID}" \
    vifu

COPY --from=build /src/target/release/vifu /usr/local/bin/vifu

USER vifu
WORKDIR /home/vifu

EXPOSE 48989
ENTRYPOINT ["vifu"]
CMD ["server", "--listen", "0.0.0.0:48989"]
