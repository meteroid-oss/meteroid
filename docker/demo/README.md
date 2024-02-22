# Demo

Requirements:
- Docker
- Disk space 64GB+
  
How to run:
```bash
docker compose -f docker/demo/docker-compose.yml --env-file .env up
```

Ports

| port  | proto | service              | description |
|-------|-------|----------------------|-------------|
| 50061 | gRPC  | meteroid-api         |             |
| 8080  | HTTP  | meteroid-api         | callback    |
| 50062 | gRPC  | metering-api         |             |
| 9000  | HTTP  | meteroid-web         |             |
| 5432  |       | PostgreSQL           |             |     
| 8123  |       | clickhouse           |             |
| 9010  |       | clickhouse           |             |
| 9009  |       | clickhouse           |             |
| 9092  |       | redpanda             |             |
| 8090  |       | redpanda-console     |             |
| 9644  |       | redpanda-console     |             |
