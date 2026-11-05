# Preview / ephemeral-environment build of the all-in-one `standalone` binary
# (meteroid gRPC + REST + workers in one process; metering only with FEATURES=metering-server).
#
# Tuned for BUILD SPEED, not runtime speed: debug profile (stripped) + cargo-chef
# dependency caching. Do NOT ship this — production images are api.Dockerfile /
# worker.Dockerfile.

FROM lukemathwalker/cargo-chef:latest-rust-1.96.0-bookworm AS chef
ARG CARGO_REGISTRIES_CRATES_IO_PROTOCOL=sparse
ENV CARGO_NET_GIT_FETCH_WITH_CLI=true
WORKDIR /opt/src

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
ARG PROTO_VERSION=21.8
# amd64 defaults so the image builds with zero build-args on a typical Coolify host.
ARG PROTO_ARCH=x86_64
ARG GRPC_HEALTH_PROBE_ARCH=amd64
# Debug build compiles ~2-3x faster than release. Override to `release` if wanted.
ARG PROFILE=dev
# Extra cargo features. Set to `metering-server` for the full variant, which runs the metering
# server in-process (the Enterprise edition's also serves analytics on the metering port).
# Empty = lean/degraded (no metering).
ARG FEATURES=""

# Preview builds don't need gdb-grade debuginfo. Dropping DWARF + stripping
# symbols shrinks the binary from ~1.8 GB to ~100 MB, speeds up linking, and
# keeps the final image small — without touching the workspace's dev profile.
ENV CARGO_PROFILE_DEV_DEBUG=0 \
    CARGO_PROFILE_DEV_STRIP=true

RUN apt-get update && \
    DEBIAN_FRONTEND=noninteractive apt-get -y install --no-install-recommends \
      curl pkg-config unzip build-essential libssl-dev openssl cmake clang wget && \
    # protoc — needed by the build scripts
    wget https://github.com/protocolbuffers/protobuf/releases/download/v${PROTO_VERSION}/protoc-${PROTO_VERSION}-linux-${PROTO_ARCH}.zip && \
    unzip protoc*.zip && mv bin/protoc /usr/local/bin && mv include/google /usr/local/include; \
    # grpc_health_probe — used by the compose healthcheck
    GRPC_HEALTH_PROBE_VERSION=v0.4.61 && \
    wget -qO/bin/grpc_health_probe https://github.com/grpc-ecosystem/grpc-health-probe/releases/download/${GRPC_HEALTH_PROBE_VERSION}/grpc_health_probe-linux-${GRPC_HEALTH_PROBE_ARCH} && \
    chmod +x /bin/grpc_health_probe; \
    apt-get clean && rm -rf /var/lib/apt/lists/*

COPY --from=planner /opt/src/recipe.json recipe.json
# Slow, rarely-invalidated layer: compile & cache all dependencies.
RUN cargo chef cook --recipe-path recipe.json --profile ${PROFILE} --package meteroid-oss-standalone ${FEATURES:+--features $FEATURES}

COPY . .

# Build the all-in-one binary. `--profile dev` emits to target/debug (cargo's
# special-case name), any other profile emits to target/<profile>.
# The Enterprise edition when ee/ is present, the OSS edition otherwise. EDITION overrides.
ARG EDITION=auto
RUN edition="$EDITION"; \
    if [ "$edition" = auto ]; then [ -d ee ] && edition=enterprise || edition=oss; fi; \
    if [ "$edition" = enterprise ]; then \
      (cd ee && CARGO_TARGET_DIR=/opt/src/target cargo build --locked -p meteroid-enterprise-standalone --bin standalone --profile ${PROFILE} ${FEATURES:+--features $FEATURES}); \
    else \
      cargo build --locked -p meteroid-oss-standalone --bin standalone --profile ${PROFILE} ${FEATURES:+--features $FEATURES}; \
    fi && \
    cp "target/$([ "$PROFILE" = dev ] && echo debug || echo "$PROFILE")/standalone" /usr/local/bin/standalone


FROM debian:stable-slim AS runtime
COPY LICENSE /usr/share/doc/meteroid/LICENSE
RUN apt-get update && \
    apt-get upgrade -y && \
    apt-get install --no-install-recommends -y ca-certificates libssl3 libpq5 && \
    rm -rf /var/lib/apt/lists/*
COPY --from=builder /usr/local/bin/standalone /usr/local/bin/standalone
COPY --from=builder /bin/grpc_health_probe /bin/grpc_health_probe

RUN groupadd --system md --gid 151 && useradd --system --gid md --uid 151 md
USER md

EXPOSE 50061 8084
CMD ["standalone"]
