# Demo

Requirements:
- Docker
- Disk space 64GB+
  
How to run:
```bash
docker compose -f docker/docker-compose.demo.yml up
```

Ports

| port  | proto | service              | description |
|-------|-------|----------------------|-------------|
| 5432  |       | PostgreSQL           |             |     
| 50061 | gRPC  | meteroid-api         |             |
| 8080  | HTTP  | meteroid-api         | callback    |
| 50062 | gRPC  | meteroid-api         |             |
| 9000  | HTTP  | meteroid-web         |             |
| 8123  |       | clickhouse           |             |
| 9010  |       | clickhouse           |             |
| 9009  |       | clickhouse           |             |
| 9092  |       | redpanda             |             |
| 8090  |       | redpanda-console     |             |
| 9644  |       | redpanda-console     |             |
| 16686 |       | jaeger               |             |
| 3000  | HTTP  | grafana              |             |
| 4317  |       | otelcol              |             |
| 4318  |       | otelcol              |             |
| 9090  | HTTP  | prometheus           |             |
| 9200  |       | opensearch           |             |
| 5601  |       | opensearch-dashboard |             |
| 4900  |       | data-prepper         |             |
| 21890 |       | data-prepper         |             |
| 21891 |       | data-prepper         |             |
| 21892 |       | data-prepper         |             |
