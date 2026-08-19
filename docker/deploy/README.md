# Build and deploy with Docker Compose

The deployment stack runs Meteroid and its PostgreSQL, ClickHouse, and Redpanda dependencies on a single Docker host. It is intended for testing and small self-hosted environments; use the Helm chart in `k8s/meteroid` for a production Kubernetes deployment.

## Build application images

The `Docker Build` GitHub Actions workflow builds the API, scheduler, metering API, and web application for AMD64 and ARM64. It runs automatically for `triton-main` pushes and can also be started manually with **Run workflow**.

Images are published under `ghcr.io/tritondatacenter` with two deployable tags:

- `triton-main` follows the latest successful branch build.
- `sha-<full-commit-sha>` identifies one immutable source revision and is the recommended deployment and rollback tag.

The workflow uses its GitHub token to publish packages. Docker Hub credentials are optional and only increase the unauthenticated pull limit for base images.

## Deploy a build

Copy `.env.example` to `.env`, replace every placeholder secret, and set the image registry and tag:

```dotenv
METEROID_IMAGE_REGISTRY=ghcr.io/tritondatacenter
METEROID_IMAGE_TAG=sha-<full-commit-sha>
```

If the GHCR packages are private, first authenticate with a token that has `read:packages` permission:

```sh
echo "$GHCR_TOKEN" | docker login ghcr.io --username <github-user> --password-stdin
```

Pull and start the selected revision:

```sh
docker compose --env-file .env pull
docker compose --env-file .env up -d --wait
docker compose ps
```

To roll back, replace `METEROID_IMAGE_TAG` with the previous full-SHA tag and repeat the pull and startup commands. Database and object-store volumes are preserved by Compose.

## Validate deployment changes

Validate changes to the deployment definition before starting it:

```sh
./test-compose.sh
```
