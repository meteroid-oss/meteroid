FROM node:22-alpine AS base


FROM base AS builder
RUN apk add --no-cache libc6-compat
RUN apk update
# Set working directory
WORKDIR /app
RUN yarn global add turbo
COPY . .
# ee/ exists only in the enterprise repository; an empty one keeps the COPY below valid in OSS.
RUN mkdir -p ee
# Prune @md/web plus @md/portal-embed (the embed loader served at /embed.js).
RUN (cd modules/web/ && turbo prune @md/web @md/portal-embed --docker)


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
COPY --link --from=builder /app/ee/ ee/
# Run all the postinstall scripts
RUN cd modules/web && pnpm install
# Re-run postinstall scripts skipped by --ignore-scripts above:
#   @md/icons: svgr-generated dist/ used as the package entry
#   @md/web:   buf-generated connect-query proto bindings under web-app/src/rpc
RUN cd modules/web && pnpm --filter @md/icons run build
RUN cd modules/web && pnpm --filter @md/web run generate:proto
# The Enterprise edition when its entry is present, the OSS edition otherwise.
RUN cd modules/web/web-app && \
    if [ -f ../../../ee/web/editions/enterprise.ts ]; then \
      (cd ../../../ee/web && pnpm run generate:proto) && \
      WEB_EDITION=../../../ee/web/editions/enterprise.ts WEB_EDITION_ROOT=../../../ee pnpm run build:js; \
    else \
      pnpm run build:js; \
    fi
# Build the same-origin embed loader (served at /embed.js by nginx below).
RUN cd modules/web && pnpm --filter @md/portal-embed run build

# nginx state for serving content
FROM nginx:stable-alpine AS runtime
COPY LICENSE /usr/share/doc/meteroid/LICENSE
RUN apk upgrade --no-cache
# Copy from build stage
COPY --from=installer /app/modules/web/web-app/dist /usr/share/nginx/html
# Serve the customer-portal embed loader same-origin at /embed.js.
COPY --from=installer /app/modules/web/packages/embed/dist/embed.global.js /usr/share/nginx/html/embed.js
COPY --from=installer /app/nginx.conf /etc/nginx/conf.d/default.conf
COPY --from=installer /app/gzip.conf /etc/nginx/conf.d/gzip.conf
COPY --from=installer /app/init-docker-env.sh /init-docker-env.sh

EXPOSE 80

RUN chmod +x /init-docker-env.sh
ENTRYPOINT ["/bin/sh", "-c", "/init-docker-env.sh && nginx -g 'daemon off;'"]
