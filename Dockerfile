##
# Build binary
##

FROM rust:1.80 as builder

RUN USER=root cargo new --bin /app
WORKDIR /app

# Prepare dependencies
COPY ./Cargo.toml ./Cargo.lock /app/
RUN cargo build --release && \
    rm src/*.rs

# Compile app
COPY ./src /app/src/
RUN rm -rf ./target/release/github-notifier* ./target/release/deps/github-notifier* && \
    cargo build --release

##
# Prepare environment
##

FROM debian:buster-slim

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
