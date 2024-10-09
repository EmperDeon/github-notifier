##
# Build binary
##
FROM lukemathwalker/cargo-chef:latest-rust-slim-bullseye AS chef
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN apt-get update && apt-get install -y --no-install-recommends locales tzdata && \
    sed -i '/ru_RU.UTF-8/s/^# //g' /etc/locale.gen && locale-gen && \
    \
    apt-get install -y --no-install-recommends \
    ca-certificates curl libssl1.1 pkg-config libssl-dev && \
    \
    rm -rf /var/lib/apt/lists/* && rm -rf /var/lib/apt/lists.d/* && apt-get autoremove -y && apt-get clean && apt-get autoclean
# Build dependencies - this is the caching Docker layer!
RUN cargo chef cook --release --recipe-path recipe.json
# Build application
COPY . .
RUN cargo build --release --bin github-notifier

##
# Prepare environment
##

FROM debian:bullseye-slim

ARG GITHUB_REF
ENV GITHUB_REF=${GITHUB_REF:-none}

ENV TZ=Etc/UTC
ENV METRICS_ADDR=0.0.0.0
ENV METRICS_PORT=3000
ENV STATE_FILE=/app/github-notifier/state.json

USER root
RUN apt-get update && apt-get install -y --no-install-recommends locales tzdata && \
    sed -i '/ru_RU.UTF-8/s/^# //g' /etc/locale.gen && locale-gen && \
    \
    apt-get install -y --no-install-recommends \
    ca-certificates curl libssl1.1 && \
    \
    rm -rf /var/lib/apt/lists/* && rm -rf /var/lib/apt/lists.d/* && apt-get autoremove -y && apt-get clean && apt-get autoclean

WORKDIR /app
COPY --from=builder /app/target/release/github-notifier /app/
RUN chown 101:101 -R /app

USER 101
EXPOSE 3000

HEALTHCHECK --interval=5s --timeout=5s CMD /bin/sh -c "/usr/bin/curl http://127.0.0.1:3000/metrics"
CMD ["/app/github-notifier"]
