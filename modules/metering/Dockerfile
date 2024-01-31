
FROM lukemathwalker/cargo-chef:latest-rust-1-bookworm AS chef
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

RUN apt-get update && \
    DEBIAN_FRONTEND=noninteractive && \
    apt-get -y install --no-install-recommends curl pkg-config unzip build-essential libssl-dev libsasl2-dev openssl cmake clang wget; \
    # Install mold
    wget https://github.com/rui314/mold/releases/download/v${MOLD_VERSION}/mold-${MOLD_VERSION}-${MOLD_ARCH}-linux.tar.gz && \
    tar xvfz mold*.tar.gz && \
    mv mold*-linux/bin/* /usr/local/bin && \
    mv mold*-linux/libexec/* /usr/libexec && \
    rm -rf mold*; \
    # Install protoc
    wget https://github.com/protocolbuffers/protobuf/releases/download/v${PROTO_VERSION}/protoc-${PROTO_VERSION}-linux-${PROTO_ARCH}.zip && \
    unzip protoc*.zip && \
    mv bin/protoc /usr/local/bin && \
    mv include/google /usr/local/include; \
    # Cleanup
    apt-get clean; \
    rm -rf /var/lib/apt/lists/*

COPY --from=planner /opt/src/recipe.json recipe.json

ARG PROFILE
ARG CI
# Build dependencies & cache
RUN cargo chef cook --recipe-path recipe.json --profile $PROFILE --package metering
# Build application
COPY . .
RUN cargo build -p metering --profile $PROFILE


FROM debian:stable-slim
ARG PROFILE
ARG TARGET_DIR=$PROFILE
RUN apt-get update && \
    apt-get install --no-install-recommends -y ca-certificates libssl3 libsasl2-2 && \
    rm -rf /var/lib/apt/lists/*
COPY --from=builder /opt/src/target/$TARGET_DIR/metering /usr/local/bin/metering

RUN groupadd --system md --gid 151 \
    && useradd --system --gid md --uid 151 md

USER md 
EXPOSE 8080
CMD ["metering"]

