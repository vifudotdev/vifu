# syntax=docker/dockerfile:1

FROM rust:1.80-bookworm AS build

WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked

FROM debian:bookworm-slim AS runtime

ARG UID=10001
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
