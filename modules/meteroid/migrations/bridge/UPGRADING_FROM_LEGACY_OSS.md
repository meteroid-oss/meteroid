# Upgrading a database from a release up to `legacy-final`

The repository's database migration history changed after the `legacy-final` tag: about ten migrations were renumbered,
two differ in content and one was ported. A database migrated by a release up to `legacy-final` still records the old
versions, so it goes through a bridge before the regular migrations run. This page is for operators of such an
installation. Databases created by a later release are not concerned.

## What happens at startup

Every Meteroid process (API, worker, standalone) runs the migrations when it starts. It first checks the server runs
PostgreSQL 18 or later (the migrations call `uuidv7()`), and refuses with nothing changed otherwise. It then takes a
database lock, so processes starting together run them one after another, then:

1. Runs the bridge, [`modules/meteroid/migrations/bridge/oss_bridge.sql`](../../modules/meteroid/migrations/bridge/oss_bridge.sql),
   at every startup. It recognizes a legacy database by a recorded old version that the current history does not have,
   or by the old Connect migration recorded without the Connect tables. On any other database (fresh, created by a later
   release, Meteroid Cloud) it returns after a few catalog lookups, changes nothing and never fails.
   On a legacy database, in one transaction, it:
   - checks that the schema changes of every old version it is about to map are present, and refuses otherwise;
   - creates the Connect tables (the old `2026-03-12-000001_meteroid_connect` migration created only two columns);
   - records the current version of each old migration. The old version rows are kept;
   - applies the two current migrations whose version an old migration already holds, so the regular migrations take
     them as applied: the `user.preferences` column and the `idx_entity_activity_user_created` index (built without
     `CONCURRENTLY`, so writes to `entity_activity` wait for it).

   Once this is done, later startups find nothing left to do and change nothing.
2. Runs the pending migrations.

The process logs `Legacy OSS migration history moved onto the current history` when the bridge changed something.

## Upgrade

1. Check the `pgmq` version (`SELECT extversion FROM pg_extension WHERE extname = 'pgmq';`). Before 1.6.0 the queue
   tables belong to the extension and `pg_dump` leaves them out, so a restored dump has no queues and no error says
   so. The `meteroid-postgres:17.4` image ships 1.5.0: copy the `pgmq--1.*--*.sql` update scripts of a
   `meteroid-postgres` 18.3 image into the server's extension directory and run `ALTER EXTENSION pgmq UPDATE TO
   '1.11.0'` before taking any dump. Older releases keep working with 1.11.0.
2. Check the PostgreSQL version (`SHOW server_version;`). Installations created before April 2026 may run PostgreSQL
   17: upgrade the server to 18 first (`pg_upgrade`, or a dump restored into a new 18 server), with the `pgmq`
   extension available on it.
3. Stop the traffic that writes to Meteroid and let the previous version empty its queues: the new migrations drop
   the `outbox_event` and `billable_metric_sync` queues, whose consumers are gone, and the new version refuses to
   start while they hold messages. Wait until `SELECT queue_name, queue_length FROM pgmq.metrics_all() WHERE
   queue_name IN ('outbox_event', 'billable_metric_sync');` shows no message, then stop it.
4. Back up the database. Rolling back means restoring the backup: the new migrations are not meant to be reverted onto
   an older binary.
5. Run the read-only [`modules/meteroid/migrations/preflight/customer_credit_ledger.sql`](../../modules/meteroid/migrations/preflight/customer_credit_ledger.sql)
   and reconcile what it lists. The `2026-10-08-110347_customer_credit_ledger` migration aborts on negative balances
   and pending credit purchases.
6. Recommended for large installations: restore the backup into a scratch database, start the new version against it
   once, and run the checks below there first.
7. Start the new version. Do not run `diesel migration run` by hand before the new version has started once. Without a
   Meteroid binary, run `psql -v ON_ERROR_STOP=1 -f modules/meteroid/migrations/bridge/oss_bridge.sql` first, with no
   Meteroid process running, then `diesel migration run`.
8. Verify:
   ```sql
   SELECT version FROM __diesel_schema_migrations
   WHERE version IN ('20260515000000', '20260828102119', '20261101100305');  -- three rows
   SELECT to_regclass('connect_oauth_app') IS NOT NULL,
          EXISTS (SELECT 1 FROM pg_indexes WHERE indexname = 'idx_entity_activity_user_created'),
          EXISTS (SELECT 1 FROM information_schema.columns
                  WHERE table_name = 'user' AND column_name = 'preferences');      -- t, t, t
   ```

## Failures

The bridge refuses only on a legacy database, and its errors leave the database unchanged; fix the cause and restart.

- `version(s) … are recorded but their schema changes are missing`: the database was altered by hand or by an
  unreleased build. Restore the schema those versions created, or contact us.
- `customer.connected_account_id is set on some rows`: set those to `NULL` (no Connect account can exist on such a
  database), then restart.
- `message(s) wait in the … queue, which this version removes`: refused before any change. Start the previous version
  until the queue is empty, then start the new one.
- `relation "pgmq.q_…" does not exist`: the database was restored from a dump taken with `pgmq` older than 1.6.0,
  which left out the queues. Go back to the old server, update `pgmq` (step 1) and dump again.
- A migration fails after the bridge: the bridge has committed, and finds nothing left to do at the next start. Fix the
  cause (usually step 5) and restart; the migrations resume at the failed one.

A process that started with an older binary during the upgrade keeps running against the new schema and may fail
requests until it is replaced. It does not run any migration: the old version rows are kept, so it finds none pending.

## Webhook endpoints from Svix

Releases up to `legacy-final` delivered webhooks through your Svix server (`SVIX_SERVER_URL`, `SVIX_JWT_TOKEN`), and
your webhook endpoints live only there. The new version delivers webhooks itself and no longer talks to Svix: until you
import the endpoints, they receive nothing. The API image ships `meteroid-svix-import` for this.

Run it once the new version has started and migrated the database (it refuses a database with pending migrations),
with `SVIX_SERVER_URL` and `SVIX_JWT_TOKEN` set on the API service. It reads the API's configuration, including
`SECRETS_CRYPT_KEY`, which encrypts the imported secrets:

```sh
docker compose run --rm meteroid-api meteroid-svix-import          # dry run: prints the plan, writes nothing
docker compose run --rm meteroid-api meteroid-svix-import --apply  # imports
```

Each Svix application belongs to the tenant whose id is its uid, as Meteroid created them; an application without such
a tenant is reported and skipped. Each of its endpoints becomes a webhook endpoint of that tenant with:

- its url, description, event type filter and rate limit. Event types Meteroid no longer sends are dropped and
  reported; an endpoint filtered on such types only is skipped, as it would otherwise receive every event;
- its signing secret. Its deliveries carry the `svix-id`, `svix-timestamp` and `svix-signature` headers next to the
  Standard Webhooks `webhook-*` ones, so your receivers keep verifying without any change;
- its custom headers;
- its state: an endpoint disabled in Svix is imported disabled.

What needs a hand is marked `TO DO` in the output:

- Svix never reveals the value of a sensitive header. Such an endpoint is imported disabled, with the header present
  and unset: enter its value on the endpoint in Meteroid, then enable the endpoint.
- An endpoint whose url this instance refuses to send to is imported disabled: change the url or the egress settings,
  then enable the endpoint.

It is safe to run again: an endpoint whose url the tenant already has, imported or created by hand, is reported as
already present and left as it is. `--apply` exits with an error when something failed; fix the cause and run it again.

Once the import is done and you have checked the endpoints in Meteroid, shut the Svix server down and remove
`SVIX_SERVER_URL` and `SVIX_JWT_TOKEN`.

## ClickHouse

The metering API migrates ClickHouse when it starts. A ClickHouse first set up by a release before 1.0.0-rc4 (0.19.x,
1.0.0-rc1 to rc3) records checksums the current V1 to V4 migrations no longer produce, and holds usage events keyed by
base62 ids: the metering API updates those checksums, copies the events to the UUID-keyed tables between V7 and V8,
and logs both. An event whose ids do not parse is not copied, and the previous tables are then kept as
`raw_events_unconverted`, to drop once reviewed. An interrupted copy starts over at the next start.
