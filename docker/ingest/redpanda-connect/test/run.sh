#!/usr/bin/env bash
# Produces events to Kafka and checks what the stand-in Meteroid accepted and what was dead-lettered.
set -euo pipefail
cd "$(dirname "$0")"
compose() { docker compose -p meteroid-ingest-test "$@"; }
trap 'compose logs connect meteroid | tail -50; compose down -v >/dev/null 2>&1' EXIT

compose up -d --wait kafka
compose up -d meteroid connect
compose exec -T kafka /opt/kafka/bin/kafka-console-producer.sh \
  --bootstrap-server localhost:9092 --topic usage-events <<'EVENTS'
{"event_id":"e1","code":"completion","customer":"ext:northwind","properties":{"tokens":1832}}
{"code":"completion","customer":"ext:northwind","timestamp":1795684364,"properties":{"tokens":412}}
{"event_id":"e3","code":"completion","customer":"ext:unknown","properties":{}}
{"event_id":"e4","code":"completion","customer":"ext:northwind","properties":{"tokens":7},"extra":"dropped"}
not json
EVENTS

read_data() { compose exec -T meteroid sh -c "cat /data/$1 2>/dev/null || true"; }
for _ in $(seq 1 60); do
  accepted=$(read_data accepted.jsonl | wc -l)
  rejected=$(read_data rejected.jsonl | wc -l)
  [[ $accepted -ge 3 && $rejected -ge 2 ]] && break
  sleep 1
done

echo "accepted:"; read_data accepted.jsonl
echo "rejected:"; read_data rejected.jsonl
[[ $accepted -eq 3 ]] || { echo "expected 3 accepted events, got $accepted"; exit 1; }
[[ $rejected -eq 2 ]] || { echo "expected 2 rejected events, got $rejected"; exit 1; }
read_data accepted.jsonl | grep -q '"event_id": "usage-events-0-1"' || { echo "offset id missing"; exit 1; }
read_data accepted.jsonl | grep -q '"timestamp": "2026-11-26T09:12:44Z"' || { echo "epoch timestamp not converted"; exit 1; }
read_data rejected.jsonl | grep -q 'Unknown customer alias' || { echo "rejection reason missing"; exit 1; }
echo "ok"
