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
- `pnpm install --prefix modules/web`

### Run the apps

- Copy the `.env.example` file to `.env`.

- Start the database with docker compose. If you intend to run the Web, you will need the "web" profile as below.
  `docker compose -f develop/docker-compose.yml --profile web up`

- Start the Rust backend
  `cargo run -p meteroid --bin server`

It will automatically run migrations. You can then apply the seed data (in /develop/data/seed.sql) through psql or the tool of your choice.

- Start the Web frontend
  `pnpm --prefix modules/web/web-app run dev`

You can now access the app at http://127.0.0.1:5147 (_not localhost_).

If you used the seed data, you can log in with the credentials found in /develop/data/README.md.
Click on the "Sandbox" tenant on the left to access the main UI.

## Development

After a pull, you should update/build the dependencies.

- `cargo build -p meteroid`
- `pnpm install --prefix modules/web`

### Updating the protobuf files

Protobuf files are found in /modules/meteroid/proto

After an update, you can rebuild rust & reinstall the web dependencies via the command above, or you can run the following commands for faster feedback:

- `cargo build -p meteroid-grpc`
- `sh ./modules/web/web-app/build-proto-web.sh`

### Updating the database models and queries

Migrations are found in :

/modules/meteroid/crates/meteroid-repository/refinery/migrations

Queries are in :

/modules/meteroid/crates/meteroid-repository/queries

After an update, make sure you have your docker daemon up and run the following command:

- `cargo build -p meteroid-repository`
