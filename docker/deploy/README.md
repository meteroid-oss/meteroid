# Docker Compose deployment

The deployment stack runs Meteroid and its PostgreSQL, ClickHouse, and Redpanda dependencies on a single Docker host. It is intended for testing and small self-hosted environments; use the Helm chart in `k8s/meteroid` for a production Kubernetes deployment.

Copy `.env.example` to `.env`, replace every placeholder secret, and start the stack:

```sh
docker compose --env-file .env up -d
docker compose ps
```

Validate changes to the deployment definition before starting it:

```sh
./test-compose.sh
```
