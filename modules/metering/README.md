## Deploy a new metering service

### In OSS

In any language, simply implement the proto from crates/metering-grpc

In rust, you can also :

- Write your implementation of crates/query-layer-interface (check crates/query-layer-tinybird as an example)
- If your metering solution supports dynamic creation of processing pipelines, implement the register-interface
- Pass your implementation(s) in apps/metering-api/src/main.rs

If you add an implementation that can benefit other, please consider contributing it to the project !

### In Cloud

TODO grpc or openapi ?
With openapi it stays stateless, no issues with lb/scaling

Implement the service following the open-api : /openapi/metering.yaml
Secure your service (ApiKey ? mtls ? signature-based like svix https://docs.svix.com/receiving/verifying-payloads/why )
Deploy your service and provide its url in the dashboard developers/metering settings

If it is provided, the system will use it instead of the included one.

If your metering implementation allows it, you can also implement a register endpoint.
This lets you receive webhook events when billable metrics are created/updated/deleted, and update your meter processing pipeline accordingly

### Alerting

TODO should we support live alerting to customers for implementations that support it ? (ex: prom)


---------------------

On app start, we : 
- setup the events table
- start the metric server
- start the event server
- start the query server


-----------------------

Community VS Enterprise
- Deduplication : 
  - community : ReplacingMergeTree (best effort, last value is kept) or in MV ?
  - enterprise/cloud : kafka + rocksdb dedupe https://segment.com/blog/exactly-once-delivery/ 
- Audit/Data storage
  - option to not save raw events