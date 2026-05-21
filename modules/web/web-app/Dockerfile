FROM node:22-alpine AS base


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
RUN yarn global add pnpm@11 turbo

COPY --from=builder /app/.gitignore .gitignore
COPY --from=builder /app/modules/web/web-app/nginx.conf nginx.conf
COPY --from=builder /app/modules/web/web-app/nginx-gzip.conf gzip.conf
COPY --from=builder /app/modules/web/web-app/init-docker-env.sh init-docker-env.sh
COPY --from=builder /app/modules/web/out/json/ modules/web/
COPY --from=builder /app/modules/web/out/pnpm-lock.yaml modules/web/pnpm-lock.yaml


# First install the dependencies (as they change less often)
RUN pnpm install --ignore-scripts --prefix modules/web

RUN apk add --no-cache libc6-compat
RUN apk update

# Copy code & run all the postinstall scripts
COPY --link --from=builder /app/modules/web/out/full/ modules/web/
COPY --link --from=builder /app/modules/meteroid/proto/ modules/meteroid/proto/
COPY --link --from=builder /app/modules/metering/proto/ modules/metering/proto/
COPY --link --from=builder /app/crates/common-grpc/proto/ crates/common-grpc/proto/
COPY --link --from=builder /app/buf.work.yaml buf.work.yaml
# Run all the postinstall scripts
RUN cd modules/web && pnpm install
# Re-run postinstall scripts skipped by --ignore-scripts above:
#   @md/icons: svgr-generated dist/ used as the package entry
#   @md/web:   buf-generated connect-query proto bindings under web-app/src/rpc
RUN cd modules/web && pnpm --filter @md/icons run build
RUN cd modules/web && pnpm --filter @md/web run generate:proto
RUN cd modules/web/web-app && pnpm run build:js

# nginx state for serving content
FROM nginx:stable-alpine AS production
# Copy from build stage
COPY --from=installer /app/modules/web/web-app/dist /usr/share/nginx/html
COPY --from=installer /app/nginx.conf /etc/nginx/conf.d/default.conf
COPY --from=installer /app/gzip.conf /etc/nginx/conf.d/gzip.conf
COPY --from=installer /app/init-docker-env.sh /init-docker-env.sh

EXPOSE 80

RUN chmod +x /init-docker-env.sh
ENTRYPOINT ["/bin/sh", "-c", "/init-docker-env.sh && nginx -g 'daemon off;'"]
