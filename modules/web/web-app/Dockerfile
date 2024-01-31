FROM node:20-alpine AS base


FROM base AS builder
RUN apk add --no-cache libc6-compat
RUN apk update
# Set working directory
WORKDIR /app
RUN yarn global add turbo
COPY . .
RUN (cd modules/web/ && turbo prune --scope=@md/web --docker)


FROM base AS installer
WORKDIR /app
RUN yarn global add pnpm turbo

COPY --from=builder /app/.gitignore .gitignore
COPY --from=builder /app/modules/web/web-app/nginx.conf nginx.conf
COPY --from=builder /app/modules/web/out/json/ modules/web/
COPY --from=builder /app/modules/web/out/pnpm-lock.yaml modules/web/pnpm-lock.yaml


# First install the dependencies (as they change less often)
RUN pnpm install --filter=@md/web --ignore-scripts --prefix modules/web

# install protoc
ARG PROTO_VERSION=21.8
ARG PROTO_ARCH
RUN apk add --no-cache libc6-compat
RUN apk update
RUN wget https://github.com/protocolbuffers/protobuf/releases/download/v${PROTO_VERSION}/protoc-${PROTO_VERSION}-linux-${PROTO_ARCH}.zip && \
    unzip protoc*.zip && \
    mv bin/protoc /usr/local/bin && \
    mv include/google /usr/local/include


# Copy code & run all the postinstall scripts
COPY --link --from=builder /app/modules/web/out/full/ modules/web/
COPY --link --from=builder /app/modules/meteroid/proto/ modules/meteroid/proto/
COPY --link --from=builder /app/crates/common-grpc/proto/ crates/common-grpc/proto/
# Run all the postinstall scripts
RUN cd modules/web && pnpm install --filter=@md/web
# Build the project
RUN cd modules/web/web-app && pnpm run build:js

# nginx state for serving content
FROM nginx:stable-alpine as production
# Copy from build stage
COPY --from=installer /app/modules/web/web-app/dist /usr/share/nginx/html
COPY --from=installer /app/nginx.conf /etc/nginx/conf.d/default.conf

EXPOSE 80

CMD ["nginx", "-g", "daemon off;"]
