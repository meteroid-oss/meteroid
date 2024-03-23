## Build

```shell
# within project root
buf generate --template=modules/meteroid/buf.gen.yaml
```

## Run

```shell
# within project root
cd modules/meteroid/gateway
go run bin/reverseproxy.go
```

## Limitations

- does not forward custom http headers (ie x-api-key, idempotency-key)
- issues with proto messages unresolved imports in the generated go code
