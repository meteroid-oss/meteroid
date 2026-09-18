# Mollie — research & integration plan

Template: **gocardless.rs** (hosted redirect, asynchronous settlement,
webhook-backed completion, dashboard/URL-managed webhooks) with
`supports_cards = true`. Two rails: **cards** (`creditcard` mandates) and
**SEPA Direct Debit** (`directdebit` mandates minted by a bank-method first
payment); PayPal mandates are a follow-up (§8).

## 1. Sources

- OpenAPI 3.1 spec (authoritative, fetched 2026-09-11):
  https://github.com/mollie/openapi → `specs.yaml` (also served at
  https://docs.mollie.com/specs.yaml). Every docs page is available as raw
  markdown at `<page-url>.md` (index: https://docs.mollie.com/llms.txt).
- Authentication: https://docs.mollie.com/reference/authentication
- Idempotency: https://docs.mollie.com/reference/api-idempotency
- Recurring payments guide: https://docs.mollie.com/docs/recurring-payments
- Create payment: https://docs.mollie.com/reference/create-payment
- Get payment / statuses: https://docs.mollie.com/reference/get-payment,
  https://docs.mollie.com/docs/handling-payment-status
- Extra (method-specific) payment fields:
  https://docs.mollie.com/reference/extra-payment-parameters
- Mandates: https://docs.mollie.com/reference/get-mandate,
  https://docs.mollie.com/reference/create-mandate
- Customers: https://docs.mollie.com/reference/create-customer
- Refunds: https://docs.mollie.com/docs/refunds,
  https://docs.mollie.com/reference/create-refund
- Chargebacks: https://docs.mollie.com/reference/get-chargeback,
  https://docs.mollie.com/reference/chargebacks-api-webhooks
- Classic webhooks: https://docs.mollie.com/reference/webhooks,
  https://docs.mollie.com/reference/payments-api-webhooks
- Errors / rate limits: https://docs.mollie.com/reference/handling-errors,
  https://docs.mollie.com/reference/rate-limiting
- Currencies: https://docs.mollie.com/docs/multicurrency
- Official SDKs (shape references only): `mollie/mollie-api-typescript`,
  `mollie/mollie-api-php`, `mollie/mollie-api-python`. **There is no official
  Rust crate**; the community `mollie`/`mollie_api` crates on crates.io are
  not used. We hand-roll `mollie-client` from the OpenAPI spec.

## 2. Auth & environments

- Base URL `https://api.mollie.com/v2/`; `Authorization: Bearer <api key>`.
  API keys come in pairs and select the mode by prefix: `test_…` / `live_…`.
  There is **no sandbox host** and no environment toggle: the key IS the mode
  (same posture as Stancer). `testmode`/`profileId` only apply to
  organization access tokens and must not be sent with an API key.
- Key validation at connect time: `GET /v2/methods` (accepts API keys;
  `/v2/organizations/me` and `/v2/permissions` do not).
- `Idempotency-Key` header: accepted on every `POST`, a UUID v4 is
  recommended but any unique string is accepted (the header is typed as a
  plain string); same key within **1 hour** replays the cached response
  (`Idempotent-Replayed: true`); different params under the same key → 400;
  concurrent duplicate → 409. Ignored on GET/PATCH/DELETE. Our
  `IdempotencyKey` strings are sent verbatim, as for Stripe and GoCardless.
- `MolliePublicData {}` — nothing public. Every payment we create names the
  connector's inbound endpoint (`{webhook_base}/webhooks/v1/{tenant}/{alias}`,
  built server-side from `METEROID_WEBHOOK_EXTERNAL_URL`, falling back to
  `METEROID_REST_API_EXTERNAL_URL`) as its `webhookUrl` (§5).
- `MollieSensitiveData { api_key, webhook_token }` — the API key and a
  server-generated random token appended to `webhookUrl` (`?token=…`) that
  authenticates the unsigned webhook pings.

## 3. Capability matrix (`MOLLIE_CAPABILITIES`)

| Field | Value | Justification |
|---|---|---|
| `supports_cards` | `true` | Card rail via `creditcard` first payment → reusable mandate. |
| `supports_mandates` | `true` | The mandate (`mdt_…`) is the off-session instrument (`sequenceType: recurring`). |
| `supports_refunds` | `false` | Same posture as every other adapter: `refund()` is `Unsupported` until the billing layer wires refunds. `POST /v2/payments/{id}/refunds` is trivial to add. |
| `supports_partial_refunds` | `false` | Implied by the above (Mollie does support partial refunds). |
| `supports_3ds` | `true` | The hosted checkout runs 3-D Secure on `first`/`oneoff` card payments; recurring charges are MIT (no 3DS). |
| `supports_disputes` | `true` | Chargebacks surface on the payment (`amountChargedBack`, `_links.chargebacks`) and are normalized to dispute events. |
| `supports_self_webhook_registration` | `false` | Webhooks need no registration (per-payment `webhookUrl`). |
| `asynchronous_settlement` | `true` | `POST /v2/payments` may return `open`/`pending`; final state arrives via webhook. |
| `supported_payment_methods` | `&[Card, DirectDebitSepa]` (Bacs is beta at Mollie: not offered) | Cards via `creditcard`; SEPA via a bank-method first payment (`ideal`, `bancontact`, `belfius`, `eps`, `kbc`, `paybybank`) → `directdebit` mandate. ACH has no Mollie mandate and is rejected. |
| `mandate_setup_mode` | `HostedRedirect` | No server-side tokenization: the customer enters the card on `_links.checkout` (Mollie Components would be `EmbeddedClientSecret`-like; not v1). |
| `webhook_replay_tolerance_secs` | `3600` | Pings carry no timestamp; a replay is harmless (idempotent re-fetch of the payment). |
| `hosted_setup_completion` | `WebhookBacked` | The payment webhook completes a hosted setup even when the return redirect is lost. |
| `supports_hosted_invoice_payment` | `true` | On an invoice or checkout the hosted `first` payment is the payment itself: completion records it onto the pre-created transaction, never charges again; pending hosted intents are swept. |

Contract fit: one **extension** is needed (see §5): Mollie webhooks are
content-free pointers ("payment `tr_…` changed"), and `parse_event` is
synchronous. A new `NormalizedEventKind::ResourceChanged { resource_ref }`
plus `WebhookOps::resolve_resource_change` (async, default `Unsupported`) lets
the dispatcher ask the adapter to read the resource back and expand it into
the existing event vocabulary. No other new variant or capability bit.

## 4. Request map (trait method → Mollie endpoint)

### `CustomerOps::create_customer`
`POST /v2/customers` `{name, email, metadata: {meteroid.customer_id,
meteroid.tenant_id}}` + `Idempotency-Key`. → `ExternalCustomerRef { cst_… }`.

### `MandateOps::initiate_mandate_setup`
1. `POST /v2/payments` `{amount, description, redirectUrl, cancelUrl,
   webhookUrl, method: "creditcard", sequenceType: "first", customerId,
   metadata}` + `Idempotency-Key`. `webhookUrl` is **required** by the
   adapter (a connector without one fails closed): a hosted checkout's
   captured payment has no sweeper and no provider id to reconcile by until
   the webhook binds it.
   - Rail: `Card` when the requested methods include `Card` (`method:
     "creditcard"`), else `Sepa` when they include `DirectDebitSepa`
     (`method: [ideal, bancontact, belfius, eps, kbc, paybybank]`). The
     customer's checkout tab decides upstream: `SetupIntentRequest.
     connection_type` / `InitiateHostedCheckoutRequest.connection_type` is a
     soft preference applied when the connection serves both rails.
   - Plain add-payment-method / invoice setup: cards `amount.value = "0.00"`
     (documented for `creditcard` and `paypal`: "No money will then be
     debited from the card"); SEPA `"0.01"` EUR — Mollie forbids zero on bank
     methods, so this is the documented verification payment. It settles to
     the merchant like any payment and is not recorded locally.
   - SEPA mandates come only from this hosted bank-verification flow: the
     customer authorises on Mollie's page and Mollie derives the IBAN from
     the transfer. No typed-IBAN form (`POST /v2/customers/{id}/mandates`):
     we don't store SEPA mandate consent evidence.
   - SEPA collects EUR only: a non-EUR checkout or charge on a SEPA mandate is
     rejected up-front.
   - Invoice-payment page (`invoice_payment` context): the first payment IS
     the invoice amount (`meteroid.invoice_id` + `meteroid.transaction_id` in
     metadata), recorded onto the pre-created Pending transaction by the
     return handler and the `MandateSetupCompleted` webhook via
     `settle_hosted_invoice_capture` (idempotent; `fetch_transaction_status`
     confirms the capture). No verification cent and no second debit.
   - Hosted CHECKOUT (`request.checkout`): `amount` = the first payment,
     captured in the same hosted flow (GoCardless `payment_request` analogue).
   - `metadata`: `meteroid.tenant_id`, `meteroid.customer_id`,
     `meteroid.connection_id`, and exactly one of `meteroid.invoice_id` /
     `meteroid.checkout_session_id` (+ `meteroid.transaction_id` for checkout).
   - `cancelUrl` = `return_url` + `&error=flow_abandoned` (GoCardless
     `exit_uri` analogue; Mollie otherwise redirects to `redirectUrl` for
     every outcome).
2. `PATCH /v2/payments/{id}` `{redirectUrl: return_url + "&payment=tr_…"}` —
   bakes the payment id into the return URL (only known after create;
   `redirectUrl` is updatable while the payment is `open`).
3. → `HostedRedirect { intent_id: tr_…, authorisation_url:
   _links.checkout.href, expires_at: expiresAt }` (cards expire after 30 min).

### `MandateOps::complete_mandate_setup(intent_id = tr_…)`
`GET /v2/payments/{tr}`:
- `status == paid` and `mandateId` present (and the mandate is `valid`) →
  `GET /v2/customers/{customerId}/mandates/{mandateId}` → snapshot
  (`external_payment_method_id = mdt_…`, `card_brand = details.cardLabel`,
  `card_last4 = details.cardNumber`, expiry from `details.cardExpiryDate`
  (`YYYY-MM-DD` on mandates), `meteroid_*` from the payment's metadata,
  `payment_request_payment = Some(tr_…)` iff the payment names a
  `meteroid.transaction_id` — a checkout's first payment; setup payments are
  never recorded). A `directdebit` mandate maps to `DirectDebitSepa` with the
  IBAN's last four digits as the account hint.
- `status ∈ {open, pending, authorized}` (an `authorized` hold can still
  expire), paid without a mandate yet, or a `pending` mandate → `Err` tagged
  `HostedSetupPending` (retryable).
- `status ∈ {failed, canceled, expired}` or mandate `invalid` →
  `Err(MandateSetup)` (terminal).

### `MandateOps::fetch_payment_method(mdt_…, cst_…)`
`GET /v2/customers/{cst}/mandates/{mdt}` → snapshot (`meteroid_*` = `None`,
mandates carry no metadata).

### `MandateOps::cancel_mandate_setup`
Default no-op (webhook-backed provider; the sweeper never runs for Mollie).

### `PaymentOps::charge_off_session`
`POST /v2/payments` `{amount, description, sequenceType: "recurring",
customerId, mandateId, webhookUrl, metadata: {meteroid.tenant_id,
meteroid.transaction_id}}` + `Idempotency-Key` (recurring payments "get
executed immediately. Issuing these requests twice can lead to double
charges"). No `redirectUrl`, no 3DS (MIT). Response `status` →

| Mollie status | `ChargeOutcome` |
|---|---|
| `paid` | `Succeeded` (`amount_received` = `amount`, `processed_at` = `paidAt`) |
| `open`, `pending`, `authorized` | `Pending` |
| `canceled` | `Cancelled` |
| `failed`, `expired` | `Failed { retryable: false, code: details.failureReason }` |

`details.failureReason` → `DeclineKind`: `insufficient_funds` →
InsufficientFunds; `card_expired`/`inactive_card` → CardExpired;
`refused_by_issuer`/`card_declined`/`invalid_*` → DoNotHonor;
`possible_fraud` → Fraud; `authentication_*` → AuthenticationRequired;
else Other.

### `ReconcileOps::fetch_transaction_status`
`GET /v2/payments/{id}` → same table onto `RemoteTransactionStatus`
(`Succeeded { amount, currency, processed_at: paidAt }`); HTTP 404 →
`Unknown`.

### `RefundOps`
`refund` / `fetch_refund` → `Unsupported` (refund observation goes through the
payment resolver, §5; no `RefundObserved` event is emitted).

### `WebhookOps`
- `register_webhook` / `unregister_webhook` / `sync_webhook_events` →
  `Unsupported`.
- `verify_signature` — the ping is unsigned; authenticated by the URL token (§5).
- `parse_events` → one `ResourceChanged { resource_ref: tr_… }` per delivery.
- `resolve_resource_change(tr_…)` → `GET /v2/payments/{tr}` (+ chargebacks
  list when `amountChargedBack` or `_links.chargebacks` is present —
  `amountChargedBack` disappears once a chargeback is reversed) → concrete
  events (§5).

## 5. Webhook map

Classic webhooks only, reduced to "payment `tr_…` changed". The
per-payment `webhookUrl` is POSTed `application/x-www-form-urlencoded`
`id=tr_…` — no signature, always the *payment* id (also for refunds and
chargebacks on that payment). Triggers: payment reaches `paid`, `authorized`,
`expired`, `failed`, `canceled`; a refund reaches `processing`/`refunded`/
`failed`; a chargeback is received. 10 retries over 26 h; 15 s response
budget. **Authentication**: our `webhookUrl` carries `?token=<webhook_token>`
(per-connector random secret); the router exposes the request query string to
the adapter as the synthetic `x-meteroid-request-query` header and
`verify_signature` constant-time compares the token. Ingest units carry no
dedup id (each ping is a fresh "re-read this payment").

Next-gen signed events are not supported: `payment.*`/`refund.*`/
`chargeback.*` event types are in closed beta at Mollie, and classic pings
cover the same state changes.

Reversal events are emitted in `occurred_at` order (the reversal store keeps
a `reversed_at` high-water mark and would drop older ones delivered late).
Every `amount.value` is parsed with the ISO 4217 exponent from
`meteroid_money::iso`; an unparseable amount or unknown currency is an error
(pgmq retry → dead-letter), never a silent `0`.

`owner_tenant_id` = the payment's `meteroid.tenant_id` metadata (cross-tenant
delivery guard).

## 6. Flows

### Recurring (save card, charge later)
1. `create_customer` → `cst_…` (once per connection).
2. Portal `SetupIntent` → `initiate_mandate_setup` → 0-amount `first`
   payment → `HostedRedirect { tr_…, checkout URL }`.
3. Frontend hosted-redirect branch (same component family as GoCardless /
   Stancer) sends the browser to Mollie's checkout; customer enters the card
   and completes 3DS.
4. Mollie redirects to `/v1/portal/mollie/return?connection=…&dest=…&payment=tr_…`
   (or `…&error=flow_abandoned` via `cancelUrl`). The return handler runs
   `complete_webhook_backed_setup`: `complete_mandate_setup` → ownership
   check (metadata must name this connection + customer; the endpoint is
   unauthenticated) → upsert the card as the customer's default. **Money
   never moves here**; it bounces back with `mollie_status = ok |
   processing | failed | abandoned`.
5. The `payment.paid` webhook ping is the source of
   truth and the lost-return backstop: `MandateSetupCompleted` → the generic
   handler upserts the method (idempotent) and, for an invoice payment,
   records the hosted first payment onto the pre-created transaction
   (`settle_hosted_invoice_capture`), never charging again.
6. Renewals: `charge_off_session` (`sequenceType: recurring`) → `Pending` or
   `Succeeded`; `payment.paid`/`failed` webhooks (or the reconcile worker)
   settle the transaction.

### Hosted checkout (mandate + first payment in one flow)
Same rails; the `first` payment carries the real first-payment amount.
Completion (`MandateSetupCompleted` with `payment_request_payment = tr_…`)
materializes the subscription against the pre-created Pending checkout
transaction, and the `PaymentSucceeded` emitted right after it (same
resolution) settles that transaction. Hosted invoice payments work the same
way: the `first` payment is the invoice amount, recorded onto the pre-created
transaction (`supports_hosted_invoice_payment`).

## 7. Open questions / gaps

- **(a) Contract extension** — `ResourceChanged` + `resolve_resource_change`
  (§3/§5). Kept minimal and defaulted to `Unsupported` for every other
  adapter.
- **(b) Next-gen payment events are beta** (opt-in via Mollie support), hence
  classic pings are the only channel; signed events are not accepted.
  When Mollie graduates them, `supports_self_webhook_registration` could flip
  to `true` behind an organization access token.
- **(c) Idempotency window is 1 hour** (Stripe: 24 h). A retry of the same
  logical charge after an hour is a new payment; the local transaction keeps
  the first `tr_…` and reconciliation settles it, but the second payment would
  be an orphan. Same exposure class as GoCardless' 30-day window inverted;
  accepted for v1.
- **(d) Synchronous status of recurring card payments** is not enumerated by
  the docs; the adapter maps whatever `status` comes back and relies on the
  webhook/reconcile for the final state.
- **(e) Mandate `pending` right after redirect**: the return handler treats
  "paid but no mandate yet" as `processing` (retry budget 3 × 2 s); the
  webhook finishes it.
- **(f) Currency**: `amount.value` carries the currency's ISO 4217 exponent
  (2 for most, 0 for `ISK`/`JPY`), taken from `meteroid_money::iso` rather
  than a private table. Test mode only supports EUR. Card currencies: AED AUD
  CAD CHF CZK DKK EUR GBP HKD HUF ILS ISK JPY NOK NZD PHP PLN RON RUB SEK SGD
  USD ZAR; Mollie rejects unsupported ones.
- **(g) Dashboard-initiated refunds/chargebacks** are visible (the classic
  ping fires on the payment), unlike Stancer.
- **(h) The classic webhook token travels in the query string**, as Mollie's
  classic mechanism prescribes, so it can land in proxy access logs. There is
  no rotation path yet short of reconnecting (which would orphan in-flight
  payments' `webhookUrl`); a rotation endpoint is a follow-up.

## 8. Follow-ups (not in v1)

- PayPal mandates (`method: paypal` first payment, zero amount allowed).
- A typed-IBAN SEPA form (`POST /v2/customers/{id}/mandates`), only once we
  store per-mandate consent evidence (timestamp, IP, mandate text version).
- Mollie Components (`cardToken`, embedded card form) as an
  `EmbeddedClientSecret`-style setup.
- `refund()` via `POST /v2/payments/{id}/refunds` once refunds are wired.
- Next-gen webhook self-registration with an organization access token.
