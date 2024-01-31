## Random event generator

Simple seeder to continuously generate and ingest events from a yaml schema.

### Usage

Simply add one or more event schema in seed.yaml, and set the number of event per second and (optional) limit

Start the metering server, then run the generator to ingest batch of events via grpc :

`cargo run -p generator`

