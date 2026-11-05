# Meteroid Helm Chart

Deploys the Meteroid API, worker, metering and web app. Each chart release pins the images of the same Meteroid
release: chart `2.0.0` runs `ghcr.io/meteroid-oss/{meteroid-api,meteroid-worker,metering-api,meteroid-web}:v2.0.0`.
These images include the Enterprise modules. Under the [Enterprise license](../../editions/enterprise/LICENSE),
they are free for companies under 50 employees and €2M in annual revenue, larger companies need a commercial license, and a
license unlocks the paid features (see Licensing). To run the open-source edition only, build the images from the repository with `EDITION=oss` and
set `<component>.image.repository`.

## Prerequisites

The chart deploys the application only. It connects to:

- **PostgreSQL** with the [`pgmq`](https://github.com/pgmq/pgmq) extension available: the migrations run `CREATE EXTENSION pgmq`.
  We publish Postgres images that include it: `ghcr.io/meteroid-oss/meteroid-postgres:18.3-cnpg` for CloudNativePG and
  `ghcr.io/meteroid-oss/meteroid-postgres:18.3-standard`.
- **Kafka**. The defaults expect SASL/SCRAM-SHA-512 (`global.kafka.*`).
- **ClickHouse** with Keeper (or ZooKeeper). The migrations create `Replicated*MergeTree` tables `ON CLUSTER`
  `global.clickhouse.clusterName`, and the ClickHouse user needs `named_collection_control`.
- **S3-compatible object storage** (`global.objectStore.*`).
- **Traefik** for the default `IngressRoute`s, or set `ingress.useIngressRoute: false` for a standard `Ingress`.

Optional: Redis (`global.redis.url`), which rate limits the API and shares caches across replicas (without it, the API is
not rate limited), and SMTP (`global.smtp.*`).

## Install

```bash
helm install meteroid oci://ghcr.io/meteroid-oss/charts/meteroid --version <version> -f values.yaml
```

Releases are listed on the [package page](https://github.com/orgs/meteroid-oss/packages/container/package/charts%2Fmeteroid).

## Configuration

See [`values.yaml`](values.yaml). The values you will set in most installs:

| Value | Description |
|---|---|
| `global.publicAppUrl`, `global.publicApiUrl`, `global.publicRestApiUrl` | Public URLs of the web app, gRPC API and REST API |
| `ingress.hosts`, `ingress.tls` | The same hosts, and their TLS secret |
| `global.secrets.existingSecretName` | Secret holding the keys below. Leave empty to have the chart create it from `global.secrets.*` |
| `global.secrets.databaseUrl`, `jwtPrivateKey`, `internalApiSecret` | Required. Use long random strings for the two secrets |
| `global.secrets.secretsCryptkey` | Required. Exactly 32 ASCII characters, e.g. `openssl rand -hex 16`. It encrypts the connector credentials stored in Postgres: keep it, as changing it makes them unreadable |
| `global.kafka.*`, `global.clickhouse.*`, `global.objectStore.uri` | Kafka, ClickHouse and object storage |
| `global.webhooks.*` | Outbound webhook delivery, e.g. an egress proxy. Private and internal destinations are refused unless `allowPrivateDestinations` is set |
| `global.smtp.host`, `from`, `customerFromAddr`, `replyTo` | Email delivery. `from` is required with a host; customer emails go out from `customerFromAddr`, by default the address of `from`. Credentials: `global.secrets.smtp.*` |
| `global.secrets.kafka.password`, `clickhouse.password`, `redis.password` | Their credentials |
| `<component>.resources`, `<component>.autoscaling` | Unset by default; autoscaling needs CPU requests |
| `<component>.extraEnv` | Extra environment variables for a component |

Components are `api`, `worker`, `metering` and `web`. The worker runs the scheduled jobs and queue consumers. Until
2.1 the chart also reads the former `scheduler` key, over `worker`: move its settings to `worker`.

## Licensing

Without a license the instance runs the free tier. To unlock the licensed features, either:

- set `global.secrets.licenseKey` (or the `license-key` key of your existing secret) to a license key, or
- activate a code in Settings > License; the instance then refreshes its license from `global.licenseServerUrl`
  (by default Meteroid's).

A license is bound to one installation, by the instance id shown in Settings > License. On an instance with several
organizations, set `global.adminOrganizationId` to the organization that manages the license.

## Development

```bash
helm lint --strict k8s/meteroid -f k8s/meteroid/ci/minimal-values.yaml
```

The chart in the repository has version `0.0.0` and `appVersion: tip`; the release workflow sets both from the release tag.
