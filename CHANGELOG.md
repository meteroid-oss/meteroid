# Changelog

From 2.0.2, the release notes are in [GitHub Releases](https://github.com/meteroid-oss/meteroid/releases),
as are those of the versions before 2.0.

## [2.0.1] - 2026-11-05

### OSS

- The metering API image includes the PostgreSQL client library its arm64 build needs: on arm64, the
  2.0.0 metering API stopped at startup with `libpq.so.5: cannot open shared object file`.

### Upgrade notes

- None. On arm64, upgrade from 2.0.0 to get a metering API that starts.

## [2.0.0] - 2026-11-04

> [!WARNING]
> **Upgrade steps required.** Follow [Upgrading to 2.0](https://docs.meteroid.com/self-hosting/upgrading-to-2-0)
> and back up your database first: the migrations cannot be reverted. Webhooks no longer go through
> Svix: import your endpoints as the guide shows.

This release brings a new baseline to Meteroid OSS, built from the same codebase as Meteroid Cloud and
Meteroid Enterprise. The open-source edition now includes a lot more, and gets bug fixes and security
patches much faster.

The official Docker images are now Meteroid Enterprise: free for companies under 50 employees and €2M
in annual revenue, with even more features. To run the open-source edition only, build the images from this repository. Details in
[Editions](https://docs.meteroid.com/self-hosting/editions).

The Helm chart is now officially maintained by our team.

Enjoy!
The Meteroid Team

### OSS

#### Credits and wallets
- Customer credit is a ledger. Each customer has one wallet per currency, made of grants with their own expiry and priority, and every movement is recorded. Wallets, grants, the ledger and top-ups are in the REST API, with `wallet.*` and `credit_grant.expired` webhooks.
- Promotional credit (goodwill, bonus credit, promotion codes) comes off before tax, like a discount. Paid credit (purchases, top-ups, overpayments) pays the invoice after tax, and the invoice PDF shows the credits applied and the amount due.
- You sell credit by charging a saved payment method or by sending an invoice. The customer Credit tab shows the balance, daily usage, grants and activity with a ledger export, and customers see their credit in the portal.

#### Billing and invoicing
- Invoices, credit notes and customer emails come in eight languages: English, French, German, Spanish, Italian, Dutch, and Brazilian and European Portuguese. Each invoicing entity has a document language and a legal footer per language, customers have an ordered list of preferred languages, and statutory tax mentions follow the seller's jurisdiction.
- B2C billing: plan versions can be priced tax-inclusive ("9.99 incl. VAT"), and a customer can be an individual with a first and last name.
- Draft invoices that are not finalized automatically are refreshed every hour with late usage, and a draft follows the customer's current billing details until it is finalized, unless you edit them on that draft. A voided invoice of the current period can be regenerated, and credit note numbers follow a pattern set per invoicing entity.
- Credit notes are in the REST API, and invoices and credit notes show their number and link a corrective invoice to the invoice it replaces.
- Customer emails are branded per invoicing entity, sent in the customer's language, and cover issued credit notes, failed payments and 3-D Secure confirmations. Each workspace has a delivery mode: Live, Test (only your team and an allowlist) or Off.

#### Subscriptions
- Cancelling a subscription immediately, or changing its plan, credits the unused part of fees paid in advance with a credit note on the original invoice.

#### Payments and refunds
- Mollie is a new payment provider for cards and SEPA Direct Debit. The card or mandate is saved, so later invoices are charged automatically.
- Refunds move money: online through Stripe, Mollie, GoCardless and Stancer, or recorded as offline refunds, each drawn on a refund credit note. Disputes and chargebacks keep the invoice paid and record the loss, while bank-side failures such as SEPA returns reopen it.
- Automatic payment retries stop after a hard decline (expired card, fraud, revoked mandate, or a bank's do-not-retry answer) and resume when the customer sets a new payment method or the card is updated. Customers can answer a 3-D Secure challenge on a renewal charge from the portal invoice page.
- Payment provider connections can be resumed during setup, disabled, deleted without losing their payments, and given new credentials in place. Adding the same card or mandate again no longer creates a duplicate payment method.
- A Test gateway simulates card and direct-debit payments (success, decline, 3-D Secure, pending) in non-production tenants, when enabled on the instance.

#### Plans, catalog and quotes
- Products can be created and archived from the products page. Archiving a feature hides it with its entitlements and frees its code, and it can be undone. Features and entitlements can be managed through the REST API.
- A feature can hold a configuration value (number, boolean, text, a choice from a list, key-value or JSON) resolved through entitlements, like any other feature. Entitlements can also be set per customer, as a default that the customer's subscriptions override.
- Draft quotes, and sent quotes nobody has signed yet, can be edited. Any quote can serve as a template, and quotes show the VAT and registration numbers of both parties.

#### Checkout and customer portal
- Purchase links are public, reusable links to buy a plan. Checkout sessions take success and cancel URLs, and the hosted checkout says whether a promotion code is valid.
- Customer portal tokens can be scoped read-only or read and manage. A browser-facing client API (`/api/client/v1`) returns the customer, their subscriptions and their entitlements, so your frontend can call Meteroid directly.

#### Usage and metering
- Billable metrics can filter events by property before aggregating (equal, not equal, in, not in)
- The metric editor previews usage live, on drafts too, with event and property discovery.
- Usage events are deduplicated at ingestion: an event id is an idempotency key per customer for 35 days, and the first copy wins.
- A subscription can bill only the events that match its usage filter (one property and one value), so one customer can hold a subscription per team or API key. Events no subscription claims are reported as unattributed usage.

#### Webhooks
- Meteroid delivers outbound webhooks itself, without a Svix server. Endpoints are managed in Settings and in the REST API, signed with Standard Webhooks headers, retried with backoff, then paused and disabled when they keep failing; every attempt is logged and can be resent. Endpoints carried over from Svix keep sending the `svix-*` headers.
- New events: `subscription.updated`, `subscription.cancelled` and `subscription.ended`, `invoice.deleted`, refunds, failed, reversed and reinstated payments, wallet movements, credit expiry and e-invoice generation.

#### Security and access
- Members can protect their account with two-factor authentication (authenticator app and recovery codes).
- Access is checked per permission, across customers, subscriptions, invoices, quotes, the catalog, events and settings. Owners hold every permission, and the built-in Member role holds all of them except managing members, roles and the organization.

#### Platform and operations
- Customer, subscription, invoice, credit note and quote lists have filters and saved views, and every list shares one layout with a rows-per-page picker. Settings open in a large dialog with side navigation, and the app reopens the last organization and tenant you used.
- Organizations and tenants can be deleted from their settings. Their data is purged in the background.
- A Sandbox tenant can be seeded with a sample business (plans, customers, subscriptions, invoices and usage). Self-hosted onboarding offers one as an optional last step.
- An optional instance bootstrap creates a user, an organization, a tenant and an API key at startup, for Docker Compose and CI.
- The per-event Segment tracking and the startup geolocation lookup are gone. Instead, an installation sends one anonymous daily report (version, deployment method, counts), which can be turned off.
- Logs and traces carry the tenant, customer, subscription and invoice ids they concern, and background work and payment provider calls are traced.

#### Fixes
- A cached API key validation is only reused for a request that presents the same secret. The hosted checkout success page no longer redirects to an address taken from its URL.
- Tax is rounded once per rate over the whole invoice, including when a draft is edited, so totals no longer drift by a cent. Conversions between currencies with different minor units use the correct number of decimals.
- A fee billed in arrears is prorated on a mid-period cancellation, and a cancellation at period end takes effect at the end of the current period. An add-on on a shorter cadence than its subscription is billed on its own cadence, and accepting a quote keeps its seat counts.
- An event at the exact boundary of two periods is billed once, metered quantities are rounded to a precision that matches their price instead of carrying aggregation noise, and zero-quantity lines are dropped.
- Invoice PDFs no longer fail on characters their font lacks, keep sub-cent unit prices, print the invoice date as the issue date, and carry the right "VAT not applicable" mention on cross-border exports.
- Charges no longer risk being taken twice after a retry, Stripe payment intents use a stable idempotency key, and a card decline at confirmation is recorded as a failed payment.
- A failing message no longer blocks the rest of its queue, failed messages back off before they are dead-lettered, and requests no longer exhaust the database pool while waiting on an external service.

### Enterprise

Each feature is marked **free** (with the Enterprise images, no license) or **license**.

#### Editions and licensing
- The official Docker images are the Enterprise build, free for companies under 50 employees and €2M in annual revenue; larger companies need a commercial license. Without a license, an installation runs one Production tenant per organization, one invoicing entity per tenant, and the free features below. A license, activated in Settings › License, unlocks the others, and you can switch between the open-source and Enterprise images on the same database.
- **Several invoicing entities and organizations** (license), which the open-source edition had up to 1.0.0-rc7.
- **License issuing** (license): issue signed software licenses to your customers, tied to their subscriptions or not, with the activation protocol their installations refresh against.

#### Credits
- **Credit units** (free): your own currency of credit, such as "API credits": you price usage in it, sell it at a price per currency, and bill usage beyond the balance as overage in money.
- **Credit packs** (free): included in plans, add-ons and subscriptions, sold or free, granted every period or once at start, with expiry and rollover.
- **Self-serve top-ups** (free): customers buy credit in the portal when you allow it.
- **Scoped credit** (license): grants for one subscription, for usage only or for some products.

#### Customer portal
- **Self-service** (free): customers change their plan, add-ons and seats, top up credit and redeem promotion codes. The open-source portal shows the same screens read-only, and customers still pay their invoices, manage payment methods and cancel.
- **Embedded portal** (license): the portal and its widgets inside your application.
- **Portal theming** (license): theme mode and corner roundness, on top of your logo and colors.

#### Billing and invoicing
- **Dunning** (free): payment reminder sequences per invoicing entity, with steps before and after the due date, audience rules, editable copy per language and per-customer overrides.
- **Tax reports** (free): the VAT return and the EC Sales List, ready to file.
- **E-invoicing** (free): invoices and credit notes are issued as EN 16931 Factur-X or XRechnung documents. Missing fields are listed before finalization, and the outcome shows on the invoice list, in the REST API and in the `invoice.accounting_pdf_generated` webhook.
- **Customer email customization** (free): which emails go out and to whom, the sender, previews and test sends, and a log of every sent or skipped email.
- **Invoice consolidation** (license): bill all of a customer's subscriptions on one invoice.
- **Minimum commitments** (license): a minimum spend per billing period on plans, subscriptions and quotes. The shortfall is billed as a true-up of the closed period, after coupons.
- **Invoice annexes** (license): usage tables built from your events, attached to invoices and editable on corrections.
- **Custom properties** (license): your own typed fields on customers, subscriptions, invoices, credit notes and quotes, usable in filters, breakdowns and CSV imports.

#### Quotes
- **Several signers** (license): collect signatures from several people on a quote.
- **Purchase orders** (license): customers add their purchase order to the quote they sign.

#### Usage controls
- **Live balances and limits** (free): live credit balances, low-balance and threshold alerts, hard limits on metered entitlements, and a real-time access check (REST and client API) that your product calls before serving a request. Meteroid never rejects usage events.
- **When credit runs out** (free): a rule on the plan (invoice the overage, deny access at zero, or invoice up to a cap then deny access), with a per-wallet override.
- **Usage costs** (license): what each customer's and subscription's usage costs over time, and a running total of the usage the next invoice will bill, in the REST API (beta).
- **Budgets** (license): cap a customer's or one subscription's usage spend with alerts or denied access. Customers can manage them in the portal.
- **Automatic top-ups** (license): buy credit automatically when a wallet runs low, within a monthly cap. Customers can manage them in the portal.

#### Intelligence
- **MRR and revenue reports** (free): recurring revenue and revenue, with their movements over time.
- **Analytics and dashboards** (license): explore any metric, break it down by customer, plan, segment or custom property, and share dashboards. An activation funnel shows how new customers reach their first paid invoice, and where they stall.
- **Customer signals and steering** (license): see which customers are at risk or ready to expand, set targets on your metrics with pacing against recent periods, and send alerts to Slack, Discord, email or a webhook.

#### Security and access
- **Single sign-on** (license): your team signs in through your identity provider (OIDC) on the domains you verify, and you can enforce it.
- **Required two-factor authentication, custom roles, security and audit logs** (license).

#### Multi-business and platform
- **Several Production tenants and organizations, and Connect** (license): Connect lets businesses on your platform bill their own customers through you. A connected account created through the REST API goes through onboarding.
- **White-label** (license): "Powered by Meteroid" is hidden from portals, checkout and emails.
- **Production hardening** (free): a Redis-backed API rate limiter and a guard that keeps outbound webhooks and alerts away from private or disallowed addresses; new organizations start with a Sandbox tenant.

### Upgrade notes

Upgrade directly from 0.19 or any 1.0.0 release candidate, with no intermediate version: follow
[Upgrading to 2.0](https://docs.meteroid.com/self-hosting/upgrading-to-2-0). Back up the
database first: the migrations are not meant to be reverted.

#### Before upgrading

- **PostgreSQL 18** is required. On an older server the processes stop at startup and change
  nothing. Installations created before April 2026 may run `meteroid-postgres:17.4`.
- **pgmq 1.6.0 or later** before any `pg_dump`: with an older `pgmq` (the 17.4 image ships
  1.5.0), a dump leaves out the queues and their messages. The guide shows how to update it.
- **Let the queues drain.** 2.0 removes the `outbox_event` and `billable_metric_sync` queues,
  and refuses to migrate, changing nothing, while they hold messages.
- **Run the credit preflight** query (`modules/meteroid/migrations/preflight/customer_credit_ledger.sql`):
  the migration that moves customer balances to the credit ledger aborts on negative balances
  and pending credit purchases.

#### Migrations

- A database of a release up to `legacy-final` first moves its migration history onto the
  current one: some migrations were renumbered. It checks the schema before changing anything.
- Positive customer balances become customer credit.
- One index on `entity_activity` is built without `CONCURRENTLY`: writes to that table wait
  while it is built.
- ClickHouse: on a ClickHouse set up before 1.0.0-rc4 (0.19, `1.0.0-rc1` to `rc3`), the metering
  API copies the usage events to the new tables. An event whose ids are not Meteroid ids is not
  copied, and the previous tables are kept as `raw_events_unconverted`.

#### Images and deployment

- The official images are the Enterprise build, free for companies under 50 employees and €2M in
  annual revenue. Without a license, an installation runs one organization, with one Production tenant per organization and one
  invoicing entity per tenant. Several invoicing entities, invoice consolidation, several
  organizations and Connect, which the open-source edition had up to 1.0.0-rc7, now need a
  license: what you already have keeps working (consolidation included), only creating more is
  refused.
- The scheduler is now the **worker**: image `meteroid-worker`, Compose service
  `meteroid-worker`, Helm values `worker`. `meteroid-scheduler` stays an alias of the image
  through 2.x, and the chart reads `scheduler:` values until 2.1.
- Images are tagged `2.0.0`, `2.0`, `2` and `latest`. The `-ee` names are deprecated, published
  for 2.0 only.
- Docker Compose: take the whole `docker/deploy` folder, which now includes the ClickHouse
  configuration under `volume/`. The images run `latest` unless `METEROID_VERSION` is set.
- Helm: the chart deploys only Meteroid (no bundled PostgreSQL, Kafka or ClickHouse) and its
  version is the Meteroid version. Its ingress defaults to Traefik `IngressRoute`s. The API and
  metering startup probes allow 30 minutes for the migrations.

#### Settings

- Removed: every `SVIX_*` setting (keep their values for the Svix import) and `ANALYTICS_API_KEY`.
- `MAILER_FROM` is required when `MAILER_SMTP_HOST` is set.
- Added: `MAILER_CUSTOMER_FROM_ADDR`, `MAILER_REPLY_TO`, `WEBHOOK_ALLOW_PRIVATE_DESTINATIONS`,
  `WEBHOOK_EGRESS_PROXY_URL`, `EGRESS_USE_PROXY_SSO`, `WEBHOOK_DELIVERY_CONCURRENCY`,
  `WEBHOOK_RETENTION_DAYS`, `METEROID_TELEMETRY` and `LICENSE_KEY`.
- `ENABLE_MULTI_ORGANIZATION` has no effect in the open-source edition; in the Enterprise images
  it needs a license with several organizations.
- An anonymous daily usage report replaces the Segment tracking, on by default in the official
  images. Turn it off with `METEROID_TELEMETRY=off` (Helm: `global.usageTelemetry: false`).

#### Webhooks

- Meteroid delivers webhooks itself and no longer uses Svix. Import your endpoints with
  `meteroid-svix-import`, in the API image: it keeps their signing secrets, and their deliveries
  keep the `svix-*` headers next to the Standard Webhooks `webhook-*` ones.
- Coming from 0.19, the payload is no longer wrapped in `data`: the object's fields are at the
  top level, and its identifier has its own field (`customer_id`, `invoice_id`...).
- The Enterprise images refuse private and loopback destinations and plain `http`, unless
  `WEBHOOK_ALLOW_PRIVATE_DESTINATIONS=true`.

#### REST API

- No operation was removed. New: credit grants, credit ledger, top-ups and wallets per customer;
  credit units and their prices; credit notes; features and entitlements on plan versions,
  products and add-ons; plan version minimums; custom property definitions and custom properties
  on invoices and credit notes; invoice refresh and XML (e-invoicing) downloads; subscription
  summary; usage costs and events per customer and subscription; webhook endpoints and
  deliveries.
- New fields in responses, among them `custom_properties` on customers, invoices and
  subscriptions, `tax_inclusive` on plans and subscriptions, and `amount_refunded` and
  `amount_reversed` on transactions. Coupons can have a `CREDIT` discount.
- New webhook event types: `subscription.updated`, `subscription.cancelled`,
  `subscription.ended`, `invoice.closed`, `invoice.deleted`, `invoice.accounting_pdf_generated`,
  `payment.failed`, `payment.reversed`, `payment.reinstated`, `refund.issued`, `refund.settled`,
  `refund.failed`, `credit_grant.expired`, `wallet.funded`, `wallet.debited`,
  `wallet.credit_recovered` and `wallet.credit_unrecovered`.
