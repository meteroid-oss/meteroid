## Configuring ceilometer

No storage backend is required for ceilometer, we're relying solely on the metering infrastructure.

- Add a sink to rabbitmq in ceilometer pipelines (`/etc/ceilometer/[pipeline, event_pipeline].yaml`)

ex:

```yaml
---
sources:
  - name: event_source
    events:
      - "*" # Make sure to filter only the relevant events
    sinks:
      - event_sink
sinks:
  - name: event_sink
    publishers:
      - notifier://?topic=meteroid.event
```
