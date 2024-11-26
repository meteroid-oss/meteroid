#!/bin/bash

## Run manually after bootstrap for now
curl -i -X DELETE http://localhost:8083/connectors/outbox-connector
curl -i -X POST -H "Accept:application/json" -H "Content-Type:application/json" \
localhost:8083/connectors \
-d '{
  "name": "outbox-connector",
  "config": {
    "connector.class": "io.debezium.connector.postgresql.PostgresConnector",
    "tasks.max": "1",
    "database.hostname": "meteroid-db",
    "database.port": "5432",
    "database.user": "meteroid",
    "database.password": "secret",
    "database.dbname": "meteroid",
    "schema.include.list": "public",
    "table.include.list": "public.outbox_event",
    "topic.prefix": "outbox.event",
    "value.converter": "org.apache.kafka.connect.json.JsonConverter",
    "plugin.name": "pgoutput",
    "transforms": "outbox",
    "transforms.outbox.type": "io.debezium.transforms.outbox.EventRouter",
    "transforms.outbox.table.expand.json.payload": "true",
    "transforms.outbox.table.fields.additional.placement": "event_type:header:event_type,tenant_id:header:tenant_id",
    "transforms.outbox.route.topic.replacement": "outbox.event.${routedByValue}",
    "transforms.outbox.table.field.event.key": "aggregate_id",
    "transforms.outbox.route.by.field": "aggregate_type",
    "transforms.outbox.table.field.event.timestamp": "created_at"
  }
}'
