## Modules

Each module is a separate group of services, with own API, database, and configuration.

Billing is the central module, with the core data model and orchestration strategy

### Billing

- Domain : Organization & Tenants, Customers, Plans, Products, Subscriptions

### Invoicing

- Domain : Invoices, Payments, Disputes, Refunds, Credit Notes, ...

### Metering

- Domain : Billable Metrics, metering config - TODO check how to link to a plan in a safe manner (maybe we can't/shouldn't separate the domain)

## Database

We require a schema per module. You can use separate db or the same instance.
We could allow cross-schema queries/functions (ex: the supabase auth) helped with foreign data wrappers
