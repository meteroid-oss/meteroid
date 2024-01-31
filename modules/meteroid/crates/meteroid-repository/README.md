## Repository

Repository is using refinery crate to run migrations, and Cornucopia to generate the typed tokio_postgres code from SQL queries.

### Cornucopia

`cargo install cornucopia`

`cornucopia --queries-path=./queries schema ./migrations/**/*.sql && cargo fmt`

For now it generates a single file, but it will change soon, cf
https://github.com/cornucopia-rs/cornucopia/pull/211

Also, we'll soon replace by the build.rs (so no need to install globally anymore)
