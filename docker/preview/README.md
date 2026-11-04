# Preview environments (Coolify)

Spin up a full, throwaway Meteroid instance straight from a git branch.
Everything is **built from source** with fast **debug** builds — for previews,
not production.

Two flavours, pick one:

| File                        | Services                                          | Metering | Footprint |
| --------------------------- | ------------------------------------------------- | -------- | --------- |
| `docker-compose.yml`        | Postgres + backend + web                          | off      | smallest / fastest build |
| `docker-compose-full.yml`   | + ClickHouse + Redpanda + Redis                   | on       | ~1.3–1.6 GB RAM |

The **lean** file mirrors the local dev flow (`docker compose ... lite` +
`cargo run -p meteroid-oss-standalone` + web): degraded mode, no metering.
Billing, customers, plans, invoicing all work; raw-usage ingestion does not.

The **full** file adds the metering stack and builds the backend with the
`metering-server` feature so metering runs in-process — memory-tuned to fit a
small box. Built from the Enterprise repository, the backend is the Enterprise
edition and also serves analytics on the metering port.

## Coolify setup

1. **New Resource → Docker Compose** (Public/Private Repository), pointing at
   this repo and branch.
2. **Docker Compose Location**: `/docker/preview/docker-compose.yml`
   (or `/docker/preview/docker-compose-full.yml`).
3. Deploy.

That's it. The `SERVICE_FQDN_*` variables in the compose tell Coolify to generate
and route the needed domains and to feed their public URLs back into the config:

- `meteroid-web` → the app UI
- `meteroid` `:50061` → the gRPC-web API (the browser calls this cross-origin)
- `meteroid` `:8084` → the REST API

The backend's CORS mirrors the request origin, so any generated preview domain is
accepted automatically. First build is slow (full Rust compile); Docker layer
caching + `cargo-chef` make later deploys much faster.

> **Requires** an instance **wildcard domain** (Settings → Configuration) so
> Coolify can mint the subdomains. Without it, assign the domains manually
> (UI → each service → Domains) — remember the `:50061` / `:8084` ports on the
> `meteroid` service — and set `VITE_METEROID_API_EXTERNAL_URL` /
> `VITE_METEROID_REST_API_EXTERNAL_URL` (web) and `METEROID_PUBLIC_URL`
> (meteroid) to those URLs.

## Local sanity check

```bash
# uncomment the `ports:` blocks in the compose file first
docker compose -f docker/preview/docker-compose.yml up --build
# UI http://localhost:3000 · gRPC-web http://localhost:50061 · REST http://localhost:8084
```

## Memory (full variant)

Tuned via each service's own knobs, not just hard limits:

- **Redpanda** `--memory 512M` (single node, dev-sized)
- **ClickHouse** `max_server_memory_usage` 512 MB + tiny caches (`clickhouse/config.xml`)
- **Redis** `--maxmemory 96mb`, no persistence
- **Postgres** `shared_buffers=64MB`
- **Backend** kafka producer buffer trimmed to 32 MiB

`mem_limit` values are safety ceilings, not expected usage — lower them for a
tighter host, but ClickHouse/Redpanda can OOM if starved.

## Notes

- **Config** lives in `preview.env` (committed dev secrets — safe, the dev
  license only validates against dev builds). Public URLs are set in the compose
  so Coolify can inject preview domains. The full variant adds
  ClickHouse/Kafka/Redis connection env inline.
- **Data** is ephemeral (object store on `/tmp`; drop the volumes for a clean slate).
- **Build knobs** (`modules/meteroid/preview.Dockerfile`): `PROFILE` (default
  `dev`) and `FEATURES` (empty for lean, `metering-server` for full).
