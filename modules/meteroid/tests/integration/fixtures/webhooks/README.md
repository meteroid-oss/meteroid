# Webhook fixtures — Stripe & GoCardless

Realistic provider webhook payloads, assembled from official documentation /
official client SDK source (not hand-guessed shapes), for use in integration
tests instead of inline fabricated JSON. All ids are sanitized to obvious test
values (`evt_test_...`, `pi_test_...`, `EV000TEST...`, `PM000TEST...`, etc.)
but the field structure, casing, and nesting match what the provider actually
sends.

Every file is standalone JSON (one Stripe `event` object, or one GoCardless
`{"events": [...]}` envelope) — load with `serde_json::from_str` /
`std::fs::read`.

## Signature schemes (for constructing `Stripe-Signature` /
`Webhook-Signature` headers in tests)

- **Stripe**: `Stripe-Signature: t=<unix_ts>,v1=<hex hmac_sha256(secret, "<ts>.<raw_body>")>`.
  Source: `docs.stripe.com` webhook signing docs, and matches
  `modules/meteroid/crates/stripe-client/src/webhook.rs` (`Signature::parse` /
  `do_validate_signature`, 5-minute tolerance).
- **GoCardless**: `Webhook-Signature: <hex hmac_sha256(secret, <raw_body>)>`.
  **No timestamp** in the scheme — GoCardless's own Java/`.NET` SDKs sign only
  the raw request body (`Mac.hmacHex(requestBody)`), and replay protection is
  left entirely to (`provider_config_id`, `provider_event_id`) DB dedup on the
  receiving end. This matches
  `modules/meteroid/crates/gocardless-client/src/webhook.rs`
  (`GoCardlessWebhook::validate_signature`) exactly.

## Stripe fixtures (`stripe/`)

| File | Provenance | Purpose |
|---|---|---|
| `payment_intent.succeeded.json` | `docs.stripe.com/api/events/object`, `docs.stripe.com/api/payment_intents/object` | Full succeeded PI: `amount_received == amount`, `latest_charge`, no `next_action`/`last_payment_error`. |
| `payment_intent.payment_failed.json` | same + `docs.stripe.com/api/payment_intents/object` (`last_payment_error` schema, incl. nested `payment_method`) | `last_payment_error` as a full **object** (code/decline_code/message/payment_method), not a string — pins the shape the `last_payment_error: Option<serde_json::Value>` field on `StripePaymentIntent` must tolerate. |
| `payment_intent.requires_action.json` | same | `next_action.type == "redirect_to_url"` variant (3DS redirect / non-SDK flow). |
| `payment_intent.requires_action.use_stripe_sdk.json` | same | `next_action.type == "use_stripe_sdk"` variant (3DS2 in-SDK flow); `use_stripe_sdk` is itself an object (`type`, `stripe_js`), not a bool/string. Two separate files because one event *type* has two structurally different `next_action` payloads. |
| `setup_intent.succeeded.json` | `docs.stripe.com/api/setup_intents/object` | Mandate/card-on-file setup completion, no `next_action`/`last_setup_error`. |
| `payment_method.attached.json` | `docs.stripe.com/api/payment_methods/object` | Card `PaymentMethod` attached to a customer; `card.{brand,last4,exp_month,exp_year}`. |
| `payment_method.automatically_updated.json` | same | Card network pushed new expiry (Account Updater); same PM id, bumped `exp_month`/`exp_year`. |
| `charge.refunded.json` | `docs.stripe.com/api/charges/object`, `docs.stripe.com/api/refunds/object` | `refunds.data` has **two partial refunds** (3000 + 2000 minor units) and the charge's own `amount_refunded: 5000` — pins per-refund amount vs. the charge's cumulative `amount_refunded` as two independently-readable numbers. |
| `charge.dispute.created.json` | `docs.stripe.com/api/disputes/object` | Full `Dispute` object incl. `evidence` (all-null, unsubmitted) and `evidence_details`. |
| `charge.dispute.funds_withdrawn.json` | same + Balance Transaction shape (general Stripe knowledge) | Same dispute, `status: "needs_response"`, now carrying a `balance_transactions` entry showing the actual debit — this is what distinguishes `funds_withdrawn` from `created` (funds are pulled from the Stripe balance immediately, before the merchant responds). |

Envelope fields on every file: `id`, `object: "event"`, `api_version`,
`created`, `livemode`, `pending_webhooks`, `request`, `type`, `data.object`
— matching `docs.stripe.com/api/events/object` exactly.

`api_version: "2023-10-16"` was chosen deliberately: Stripe removed the
deprecated `PaymentIntent.charges` list on **2022-11-15** in favor of
`latest_charge` (confirmed via Stripe's own changelog entry
`docs.stripe.com/changelog/2022-11-15/deprecates-charges-auto-expand` and
related community reporting). **Correction vs. the brief**: the two fields
never co-exist in a single real payload on any post-2022-11-15 API version, so
`payment_intent.succeeded.json` carries `latest_charge` only, not both. If a
fixture with the legacy `charges` list is still wanted (e.g. to test a tenant
pinned to a pre-2022-11-15 `api_version`), it needs a second, older-api-version
fixture — flagged below under Gaps rather than fabricated here.

`payment_method.attached` is a real, current Stripe event type, but note it is
**not** in this codebase's `STRIPE_PAYMENT_WEBHOOKS` self-registration list in
`stripe-client/src/webhook.rs` (only `payment_method.updated`, `.detached`,
and `.automatically_updated` are registered) — worth a deliberate check by the
review, since the fixture exists but nothing subscribes to it today.

## GoCardless fixtures (`gocardless/`)

| File | Provenance | Purpose |
|---|---|---|
| `payments_confirmed.json` | `developer.gocardless.com` mandate/billing-request event examples + official SDK `Event` schema (see below) | Baseline single-event envelope, `links.payment` populated, `resource_metadata` with `meteroid.transaction_id`. |
| `payments_failed.json` | same, `details.reason_code: "AM04"` is the ISO 20022 SEPA return code for insufficient funds | `details.cause`/`reason_code` populated for a bank-side failure, `resource_metadata` populated. |
| `payments_charged_back.json` | action name confirmed via GoCardless's own enumerated action list (see Gaps for the `cause` value caveat) | `action: "charged_back"` — a customer-initiated dispute/reversal at the bank, `resource_metadata` populated. |
| `payments_late_failure.json` | same | `action: "late_failure_settled"` — **not** `"late_failure"` (see Gaps: correction vs. the brief). Matches the existing `LATE_FAILURE_SETTLED` constant and comment in `gocardless-client/src/webhook.rs`/`meteroid-store/src/adapters/payment/gocardless.rs` ("a FAILURE despite the name"), `resource_metadata` populated. |
| `mandates_active.json` | `developer.gocardless.com/mandates/responding-to-mandate-events/` pattern (`cause: "mandate_activated"`, `scheme`) | Mandate activation after Billing Request Flow completion, `resource_metadata` populated. |
| `mandates_cancelled.json` | same doc, verbatim `details` shape for a bank-closed-account cancellation | Mandate revoked at the bank, `resource_metadata` populated. |
| `billing_requests_fulfilled.json` | `developer.gocardless.com/billing-requests/responding-to-billing-request-events/` (event example reproduced almost verbatim, ids sanitized) | `links.billing_request` + `links.customer` + `links.mandate_request_mandate` (a mandate-only/mandate+payment BR outcome, per the brief's explicit ask — the docs' own worked example uses `payment_request_payment` instead since it was a payment-only BR). Note: `metadata: {}`, no `resource_metadata`. |
| `batch_multi_event.json` | composed from the same primitives, per `developer.gocardless.com/getting-started/stay-up-to-date-with-webhooks-v2/` (confirms a single `{"events":[...]}` POST batches multiple events) | One envelope, three events: `payments.confirmed` (payment A) → `mandates.active` → `payments.confirmed` (payment B, different id/tx). Exercises per-event iteration and dedup independent of delivery batching. |
| `gocardless_batch_poison_then_valid.json` | constructed for settlement lookup edge-case test (modules/meteroid/tests/integration/test_webhook.rs:469) | Two events: first carries `meteroid.transaction_id` on the top-level `metadata` field (non-standard shape), second is a normal `payments.confirmed`. Tests that a malformed event does not block subsequent valid settlements via provider-id correlation. |
| `gocardless_confirmed_empty_metadata.json` | constructed for settlement lookup edge-case test (modules/meteroid/tests/integration/test_webhook.rs:328) | A `payments.confirmed` event with `metadata: {}` and `resource_metadata: {}` (no transaction id anywhere). Tests that settlement can still succeed when no meteroid transaction id is present, relying on provider-id (`links.payment`) correlation. |

Envelope/event fields: `events[]`, each with `id` (`EV...`), `created_at`,
`resource_type`, `action`, `links`, `details` (`origin`/`cause`/`description`/
`scheme`/`reason_code`), `metadata`, `resource_metadata` — matching
`developer.gocardless.com` webhook docs and, for the `metadata` vs.
`resource_metadata` split specifically, the official GoCardless client SDKs
(see next section).

### The `metadata` vs. `resource_metadata` question — **answered, load-bearing**

The brief flagged this as needing verification for the settlement bug. It is
now doc/SDK-confirmed, from three independent official GoCardless client
libraries (all Crank-generated from GoCardless's own API spec, so they encode
the provider's actual contract, not third-party guesswork):

- **`gocardless-dotnet`**
  (`GoCardless/Resources/Event.cs`, `github.com/gocardless/gocardless-dotnet`):
  - `Metadata`: *"The metadata that was passed when making the API request
    that triggered the event (for instance, cancelling a mandate). **This
    field will only be populated if the `details[origin]` field is `api`
    otherwise it will be an empty object.**"*
  - `ResourceMetadata`: *"The metadata of the resource that the event is for.
    For example, this field will have the same value of the `mandate[metadata]`
    field on the response you would receive from performing a GET request on
    a mandate."*
- **`gocardless-pro-go`** (`event_service.go`): `Event` struct has both
  `Metadata map[string]interface{}` (`json:"metadata,omitempty"`) **and**
  `ResourceMetadata map[string]interface{}` (`json:"resource_metadata,omitempty"`)
  as distinct top-level fields.
- **`gocardless-pro-python`** (`gocardless_pro/resources/event.py`): exposes
  both `event.metadata` and `event.resource_metadata` as separate attributes
  reading from separate keys in the raw API response.

(The older `gocardless-pro-java` javadoc, pinned at 3.7.0, only documents
`getMetadata()` — no `getResourceMetadata()` — which is consistent with
`resource_metadata` being a field GoCardless added to the API after that SDK
version was cut, not evidence that it doesn't exist.)

**Conclusion**: for `payments.confirmed` / `.failed` / `.charged_back` /
`.late_failure_settled` events, `details.origin` is `bank` or `gocardless`
(the transition is bank/scheme-driven), essentially never `api`. The top-level
`metadata` field **will almost always be empty in production** for such
events, per GoCardless's API semantics. The `meteroid.transaction_id` that
`charge_off_session` actually writes (via `CreatePayment.metadata`,
`meteroid-store/src/adapters/payment/gocardless.rs` line ~370) lands on the
**Payment resource's own metadata**, which webhooks surface via `resource_metadata`.

The `gocardless-client/src/webhook.rs::Event` struct now deserializes both
`metadata` and `resource_metadata` (lines 36-42), and `normalize_payment_event`
in `meteroid-store/src/adapters/payment/gocardless.rs` reads
`resource_metadata` first with a fallback to `metadata` (lines 837-841).
Most GoCardless fixtures here include a realistic `resource_metadata` object
alongside an empty `metadata: {}`, matching production behavior where the
transaction id lives in the resource metadata. Exceptions: `billing_requests_fulfilled.json`
includes `metadata: {}` with no `resource_metadata` key, and the edge-case fixtures
(`gocardless_batch_poison_then_valid.json` and `gocardless_confirmed_empty_metadata.json`)
test scenarios where resource_metadata is absent or malformed.

## Inventory of pre-existing (hand-crafted) payloads found during this pass

Read-only references, not modified:

- `modules/meteroid/tests/integration/test_webhook.rs` — inline Stripe
  `payment_intent.succeeded` JSON via `serde_json::json!`; minimal (no
  `api_version`/`livemode` on the event envelope, no `latest_charge`), built
  purely to exercise signature verification + dedup + a >4KB body, not shape
  fidelity. Not wrong, just partial — a fine candidate to eventually swap for
  `stripe/payment_intent.succeeded.json` plus its own metadata overrides.
- `modules/meteroid/crates/stripe-client/src/webhook.rs` — no payload tests
  beyond `Signature::parse`; no fabricated event bodies to reconcile.
- `modules/meteroid/crates/gocardless-client/src/webhook.rs` tests (
  `parses_payment_event`) and
  `modules/meteroid/crates/gocardless-client/tests/gocardless_sandbox.rs`
  (`webhook_envelope_parses`) — both put `meteroid.transaction_id` /
  `meteroid.scenario` on the event's top-level `metadata`, which (see above)
  is the one field real GoCardless payment-transition webhooks essentially
  never populate. Structurally realistic otherwise (correct envelope, links,
  details).
- `modules/meteroid-store/src/adapters/payment/gocardless.rs` unit tests
  (`parse_event_payments_confirmed_succeeds`, `parse_event_payments_failed`,
  `parse_event_mandates_active`, `parse_event_mandates_cancelled`) — same
  `metadata` vs. `resource_metadata` issue; these are the tests that would
  need to start reading from `resource_metadata` (once the `Event` struct
  parses it) for a settlement-lookup fix to be verifiable.
- `modules/meteroid/tests/integration/subscription/payment_webhook_settlement.rs`
  — payloads here (`succeeded_payload`, the inline `payment_failed` JSON) are
  for the **Mock** connector (`kind: "payment_succeeded"` /
  `"payment_failed"`), not Stripe or GoCardless at all — an intentionally
  provider-agnostic synthetic shape, out of scope for this fixture set.

## Gaps — could not fully verify against primary docs this session

- **`details.cause` for `charged_back` and `late_failure_settled` payment
  events**: I could not pull a verbatim documented (or GoCardless-CLI-trigger)
  example JSON for either action's exact `cause` string — `developer.gocardless.com`'s
  API reference and CLI-trigger pages are JS-rendered and returned HTTP 404 to
  every fetch attempt (direct and via a text-extraction proxy) during this
  session; only the *action name* enum (confirmed: `charged_back` and
  `late_failure_settled` are both real, distinct actions — the latter matches
  this codebase's own `LATE_FAILURE_SETTLED` constant) came through, via
  third-party integration-guide search snippets, not GoCardless's own docs
  page content. `payments_charged_back.json` uses `cause: "chargeback"` and
  `payments_late_failure.json` uses `cause: "late_failure_settled"`
  (mirroring the action name, following the same pattern as the
  doc-confirmed `active` → `cause: "mandate_activated"` pair) — **both are
  best-effort, not doc-verified**. If exact fidelity on `cause` matters for
  test assertions, get a live sample via `gc trigger payment_charged_back` /
  the dashboard's "send test webhook" tool against a real sandbox account, or
  re-attempt the docs fetch with a browser-rendering tool.
- **Correction vs. the brief**: the brief asked about `action`/`cause` naming
  for "late failure" and suggested `"failed"` with `cause: "late_failure"` as
  a possibility. The real action is `late_failure_settled` (not `late_failure`
  or `failed`) — this codebase already has it right
  (`action::LATE_FAILURE_SETTLED` in `gocardless-client/src/webhook.rs`), and
  `payments_late_failure.json` follows the code, not the brief's guess.
- **`resource_metadata` deserialization in billing_requests_fulfilled**: the
  fixture includes `metadata: {}` with no `resource_metadata` key, matching
  an actual billing request fulfillment payload. Most payment fixtures include
  `resource_metadata` to support testing the resource-metadata-first lookup
  path.
- **Stripe legacy `charges` list on PaymentIntent**: not represented in any
  fixture (see the api_version note above) since it doesn't coexist with
  `latest_charge` on any real, current-era payload. If pre-2022-11-15
  behavior needs coverage, that's a separate, deliberately-old-api-version
  fixture, not added here to avoid presenting a payload shape Stripe would
  never actually send today.
- **`api_version` on Stripe fixtures**: set to a plausible mid-2023 value
  (`"2023-10-16"`) for internal consistency across all ten Stripe fixtures;
  I did not verify that this exact literal string is a real, still-current
  Stripe API version string (Stripe version identifiers are dated and
  numerous) — treat it as "a recent, `latest_charge`-era version" rather than
  a specific pinned value to assert on.
