
FROM lukemathwalker/cargo-chef:latest-rust-1.96.0-bookworm AS chef
ARG CARGO_REGISTRIES_CRATES_IO_PROTOCOL=sparse
ENV CARGO_NET_GIT_FETCH_WITH_CLI=true
WORKDIR /opt/src

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
ARG MOLD_VERSION=1.11.0
ARG PROTO_VERSION=21.8

ARG MOLD_ARCH
ARG PROTO_ARCH
ARG GRPC_HEALTH_PROBE_ARCH

RUN test -n "$MOLD_ARCH" || (echo "MOLD_ARCH not set" && false)
RUN test -n "$PROTO_ARCH" || (echo "PROTO_ARCH not set" && false)
RUN test -n "$GRPC_HEALTH_PROBE_ARCH" || (echo "GRPC_HEALTH_PROBE_ARCH not set" && false)

RUN apt-get update

# install needed packages
RUN DEBIAN_FRONTEND=noninteractive && \
    apt-get -y install --no-install-recommends curl pkg-config unzip build-essential libssl-dev openssl cmake clang wget

# Install mold
RUN wget https://github.com/rui314/mold/releases/download/v${MOLD_VERSION}/mold-${MOLD_VERSION}-${MOLD_ARCH}-linux.tar.gz && \
    tar xvfz mold*.tar.gz && \
    mv mold*-linux/bin/* /usr/local/bin && \
    mv mold*-linux/libexec/* /usr/libexec && \
    rm -rf mold*

# Install protoc
RUN wget https://github.com/protocolbuffers/protobuf/releases/download/v${PROTO_VERSION}/protoc-${PROTO_VERSION}-linux-${PROTO_ARCH}.zip && \
    unzip protoc*.zip && \
    mv bin/protoc /usr/local/bin && \
    mv include/google /usr/local/include

# Install grpc health checker
RUN GRPC_HEALTH_PROBE_VERSION=v0.4.61 && \
    wget -qO/bin/grpc_health_probe https://github.com/grpc-ecosystem/grpc-health-probe/releases/download/${GRPC_HEALTH_PROBE_VERSION}/grpc_health_probe-linux-${GRPC_HEALTH_PROBE_ARCH} && \
    chmod +x /bin/grpc_health_probe

# Cleanup
RUN apt-get clean; \
    rm -rf /var/lib/apt/lists/*

COPY --from=planner /opt/src/recipe.json recipe.json

ARG PROFILE
ARG CI
# Build dependencies & cache
RUN cargo chef cook --recipe-path recipe.json --profile $PROFILE --package meteroid-oss-api
# Build application
COPY . .

# Inject production license public key (if provided); only the Enterprise edition verifies licenses
ARG LICENSE_PUBLIC_KEY_PEM
RUN if [ -n "$LICENSE_PUBLIC_KEY_PEM" ] && [ -d ee/crates/enterprise-license ]; then \
      echo "$LICENSE_PUBLIC_KEY_PEM" > ee/crates/enterprise-license/keys/public.pem; \
    fi

# Reported by the anonymous instance report (ADR 0010); a release build passes its tag.
ARG METEROID_VERSION

# The Enterprise edition when ee/ is present (this repository), the OSS edition otherwise
# (its public mirror). EDITION=oss|enterprise overrides.
ARG EDITION=auto
RUN edition="$EDITION"; \
    if [ "$edition" = auto ]; then [ -d ee ] && edition=enterprise || edition=oss; fi; \
    if [ "$edition" = enterprise ]; then \
      cd ee && CARGO_TARGET_DIR=/opt/src/target cargo build --locked -p meteroid-enterprise-api --bin meteroid-api --bin meteroid-svix-import --profile $PROFILE; \
    else \
      cargo build --locked -p meteroid-oss-api --bin meteroid-api --bin meteroid-svix-import --profile $PROFILE; \
    fi


FROM debian:stable-slim AS runtime
# Official images send the anonymous daily report (docs.meteroid.com/self-hosting/telemetry); METEROID_TELEMETRY=off stops it.
ENV METEROID_TELEMETRY=on \
    METEROID_DEPLOYMENT=docker
ARG PROFILE
ARG TARGET_DIR=$PROFILE
RUN apt-get update && \
    apt-get upgrade -y && \
    apt-get install --no-install-recommends -y ca-certificates libssl3 libpq5 && \
    rm -rf /var/lib/apt/lists/*
COPY --from=builder /opt/src/target/$TARGET_DIR/meteroid-api /usr/local/bin/meteroid-api
# Run once by instances upgrading from a release that delivered webhooks through Svix.
COPY --from=builder /opt/src/target/$TARGET_DIR/meteroid-svix-import /usr/local/bin/meteroid-svix-import
COPY --from=builder /bin/grpc_health_probe /bin/grpc_health_probe

RUN groupadd --system md --gid 151 \
    && useradd --system --gid md --uid 151 md

USER md

EXPOSE 50061
EXPOSE 8080

CMD ["meteroid-api"]
