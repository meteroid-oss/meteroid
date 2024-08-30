# Contributing Guide

## Building from source

This project uses

- [Cargo](https://doc.rust-lang.org/cargo/getting-started/installation.html)
- [pnpm](https://pnpm.io/installation)
- [protoc](https://grpc.io/docs/protoc-installation/)
- [docker-compose](https://docs.docker.com/compose/install/)

Make sure to install them before proceeding, and that **your docker daemon is running**.

### Version Requirements

- Postgres >= 12
- Rust >= 1.74
- Node >= 20
- pnpm >= 8
- protoc >= 3.17

### Install the dependencies & build

- `cargo build -p meteroid`
- if you need the metering api : `cargo build -p metering`
- `pnpm install --prefix modules/web`

### Run the apps

- Copy the `.env.example` file to `.env`.

- Start the database with docker compose. If you intend to run the Metering app as well, you will need the "metering"
  profile as follows:
  `docker compose -f docker/develop/docker-compose.yml --env-file .env --profile metering up`

- Start the Rust backend
  `cargo run -p meteroid --bin meteroid-api`

It will automatically run migrations. You can then apply the seed data (in docker/develop/data/seed.sql) through psql or
the tool of your choice.

- Optionally start the Metering Rust backend
  `cargo run -p metering --bin metering-api`

- Start the Web frontend
  `pnpm --prefix modules/web/web-app run dev`

You can now access the app at http://127.0.0.1:5147 (_not localhost_).

If you used the seed data, you can log in with the credentials found in docker/develop/data/README.md.
Click on the "Sandbox" tenant on the left to access the main UI.

## Development

After a pull, you should update/build the dependencies.

- `cargo build -p meteroid`
- `cargo build -p metering`
- `pnpm install --prefix modules/web`

### Updating the protobuf files

Protobuf files are found in /modules/meteroid/proto

After an update, you can rebuild rust, reinstall the web dependencies and generate from proto via the command above, or
you can run the following commands for faster feedback:

- `cargo build -p meteroid-grpc`
- for metering: `cargo build -p metering-grpc`
- `pnpm --prefix modules/web/web-app run generate:proto`

### Database Migrations

To add new migration following steps are needed (executed from the project root):

- make sure the database server is running
- make sure diesel_cli is installed : `cargo install diesel_cli --no-default-features --features postgres`
- create the migration file : `diesel migration generate <migration_name>`. Generates empty migrations file under
  `modules/meteroid/migrations/diesel`
- add sql code to the generated migration files
- apply the migration : `diesel migration run`. Applies the migration(s) and regenerates the schema.rs file.

On meteroid_api startup the un-applied migrations run automatically.
