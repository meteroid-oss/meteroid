# Stancer — research & port plan

Port of PR #1240 (ref `stancer-fork/feat/stancer-plugin`, built on the OLD
`PaymentProvider` trait) onto the NEW `PaymentConnector` architecture.
Template: **gocardless.rs** (hosted redirect, async settlement, no
self-registered webhooks) — with `supports_cards = true` and **no webhooks at
all** (polling-primary).

## 1. Sources

- OpenAPI 3.1 spec (authoritative, fetched 2026-08-21):
  https://docs.stancer.com/api/openapi.json (rendered at
  https://docs.stancer.com/api/redoc.html and …/swagger.html)
- Auth / key model: https://www.stancer.com/documentation/api/ ("Authentication"
  part: HTTP Basic, secret key as username, no password; keys `sprod_…` live,
  `stest_…` test)
- Hosted payment webpage behaviour:
  https://www.stancer.com/documentation/fr/api/.parts/payment-webpage/
  (iframe-able, postMessage on success, auto-redirect to `return_url` after ~3s)
- **Primary shape reference**: the hand-rolled `stancer-client` crate from PR
  #1240 (`git show stancer-fork/feat/stancer-plugin:modules/meteroid/crates/stancer-client/src/…`).
  Its shapes match the OpenAPI spec exactly (verified field-by-field) and it
  carries live-verified behaviour notes (framing requires `return_url`; `auth:
  false` is rejected — omit the field to skip 3DS; settlement is async with no
  push).

Spec-verified negative result: the OpenAPI contains **zero** occurrences of
webhook / notification / callback / hook / hmac. Disputes are `GET`-only.
The contributor's claim "Stancer has no webhook mechanism" is confirmed.

## 2. Auth & environments

- HTTP **Basic** auth: secret key as username, empty password
  (`stancer-client/src/client.rs` already does this).
- One base URL for both modes: `https://api.stancer.com/v2/`. Test vs live is
  selected by the key itself (`stest_…` / `sprod_…`) — there is **no
  environment toggle** and no sandbox base URL (unlike GoCardless).
- Key validation: no dedicated endpoint; `GET /v2/ping` returns
  `{mode, company, account}` and fails on a bad key (fork's `ConnectStancer`
  handler already pings before persisting — keep that).
- `StancerPublicData {}` — empty (no publishable key, no account id; the
  hosted-page URL comes fully formed from the payment-intent response).
- `StancerSensitiveData { api_secret_key: String }` — the only credential.
  **No `webhook_secret` field** (no webhooks exist).
- Sandbox-default guardrail (GC's `default_environment` pattern) has no
  equivalent: mode rides on the key. Optional hardening: warn at connect time
  when `ping.mode` is live, or when the key prefix is neither `stest_` nor
  `sprod_`.

## 3. Capability matrix (`STANCER_CAPABILITIES`)

| Field | Value | Justification |
|---|---|---|
| `supports_cards` | `true` | Card provider; saved `card_…` token charged via `POST /v2/payments/`. |
| `supports_mandates` | `true` | Saved card is a reusable off-session token (hosted intent flow). |
| `supports_refunds` | `true` | `POST /v2/refunds/` `{payment, amount?}` (spec). Client method must be added. |
| `supports_partial_refunds` | `true` | `amount` optional on `RefundCreate`; any positive amount accepted. |
| `supports_3ds` | `true` | Hosted page runs the 3DS challenge (`threeds: required` on the intent). Off-session charges omit `auth` → no 3DS (live-verified; `auth: false` is rejected by validation). |
| `supports_disputes` | `false` | `GET /v2/disputes/` exists but there is no push channel; no dispute lifecycle in v1 (see §7). |
| `supports_self_webhook_registration` | `false` | No webhook mechanism at all (spec-verified). `register/unregister/sync` → `Unsupported`. |
| `asynchronous_settlement` | `true` | Charge lands as `to_capture`/`capture_sent` and resolves to `captured` later, with nothing pushed — reconcile worker is the settlement path. |
| `supported_payment_methods` | `&[Card]` | v1 card-only. Stancer also has a SEPA rail (`/v2/sepa/`, `/v2/mandates/`) — deferred follow-up. |
| `mandate_setup_mode` | `HostedRedirect` | No client-side tokenization SDK; the only PCI-safe capture is the hosted payment page (`payment_intents` → `url`). |
| `webhook_replay_tolerance_secs` | `3600` | Moot (no webhooks; `verify_signature` always rejects), but `assert_capabilities_consistent` requires `> 0` — use GC's value with a comment. |

Contract-fit: **everything maps onto the existing vocabulary** — no new
`MandateSetupInstruction` / `ChargeOutcome` / `NormalizedEventKind` variant and
no new capability bit is needed. (See §7 for the soft spots.)

## 4. Request map (trait method → Stancer endpoint)

Client crate status: `create_customer`, `get_customer`, `get_card`,
`create_payment`, `get_payment`, `create_payment_intent`, `get_payment_intent`,
`ping` **exist**. Must be **added**: `update_payment_intent`
(`PATCH /v2/payment_intents/{id}`), `create_refund` (`POST /v2/refunds/`),
`get_refund` (`GET /v2/refunds/{id}`), optionally
`list_payment_refunds` (`GET /v2/payments/{id}/refunds`).

### `CustomerOps::create_customer`
`POST /v2/customers/` — `{name, email, mobile, external_id}`.
`external_id` (≤36 chars, **unique**) = `customer.id.as_base62()` — the only
correlation slot (no metadata map on customers). → `ExternalCustomerRef`.
Idempotency: no key field / no documented `Idempotency-Key` header; the unique
`external_id` makes a duplicate create fail — on a 4xx conflict during retry,
resolve by `GET /v2/customers/?…` lookup (implementation detail, see §7).

### `MandateOps::initiate_mandate_setup`
1. `POST /v2/payment_intents/` — `{amount: 0, currency, customer,
   methods_allowed: ["card"], capture: false, threeds: "required",
   return_url, metadata}` (fork verified `amount: 0` accepted; schema minimum
   is 0). `metadata` (payment_intents DO have a metadata map):
   `meteroid.tenant_id`, `meteroid.customer_id`, `meteroid.connection_id`,
   plus exactly one of `meteroid.invoice_id` / `meteroid.checkout_session_id`
   (mirrors the GC pattern; no 3-property cap here).
2. `PATCH /v2/payment_intents/{id}` — `{return_url: "<rest_api_url>/v1/portal/stancer/return?connection=<cid>&intent=<pi_…>&dest=<dest>"}`
   — bakes the intent id into the return URL (the id is only known after
   create; `return_url` is patchable per `PaymentIntentUpdate`).
3. → `MandateSetupInstruction::HostedRedirect { intent_id: pi.id,
   authorisation_url: pi.url, expires_at: None }` (no expiry in the response).

`return_url` is **required** even for iframe embedding (live-verified: the
hosted page refuses to be framed without it).
Intent creation has **no** idempotency field (`order_id` is non-unique, and
`unique_id` exists only on `/v2/payments/`); a duplicate intent on retry is
harmless — `amount: 0`, nothing moves, abandoned intents just idle.
Checkout context (`request.checkout`): **not** collected in-flow in v1 — see
§6/§7; the intent stays `amount: 0` and the first payment is a server-side
off-session charge after completion (fail-closed against lost returns).

### `MandateOps::complete_mandate_setup(intent_id)`
`GET /v2/payment_intents/{id}` → `.card` (`card_…`) is the authoritative
"done" signal (fork's live-verified heuristic — don't key on `status`, whose
terminal value for a 0-amount flow is unobserved). `.card` absent + status
`canceled`/`unpaid` → failed; `.card` absent otherwise → not complete yet
(caller may retry briefly — the redirect can beat the intent's own update).
Then `GET /v2/cards/{card_id}` for brand/last4/exp →
`PaymentMethodSnapshot { external_payment_method_id: card_id,
payment_method_type: Card, card_* from the card, meteroid_* from
intent.metadata, payment_request_payment: intent.payment }` (`.payment` is
populated only if the intent carried an amount; `None` in the v1 0-amount flow).

### `MandateOps::fetch_payment_method`
`GET /v2/cards/{id}` → snapshot (fork's `get_payment_method_from_provider`
verbatim; `meteroid_*` fields `None` — cards carry no metadata).

### `PaymentOps::charge_off_session`
`POST /v2/payments/` — `{amount, currency, customer, card,
unique_id: <idempotency_key ≤36 chars>, capture: true}` and **omit `auth`
entirely** (sending `auth: false` is rejected; omission is what skips 3DS —
live-verified). `unique_id` is Stancer's idempotency mechanism on this
endpoint (unique, unicity-checked; `order_id` is the non-unique cousin —
don't use it for dedup). `transaction_id.as_base62()` fits ≤36.
The payment resource has **no metadata map** — correlation is by the returned
`paym_…` id (synchronous call, caller knows the transaction).

Status → `ChargeOutcome` (from `StatusCode` enum + fork's
`stancer_payment_to_domain`):

| Stancer status | Outcome |
|---|---|
| `captured` | `Succeeded` (amount = requested; `processed_at = now`) |
| `authorized`, `to_capture`, `capture_sent`, `null` | `Pending` (the normal initial state) |
| `refused`, `failed`, `expired` | `Failed { retryable: false, code: response }` |
| `canceled` | `Cancelled` |
| `disputed` | treat as `Succeeded` (funds were captured; dispute lifecycle out of scope v1) |

`response` is an ISO-8583-style network code (`"00"` = approved); map the
common decline codes to `DeclineKind` (`51`→InsufficientFunds,
`54`→CardExpired, `05`→DoNotHonor, `59`→Fraud, else Other) — best-effort,
`Other` fallback. Never returns `RequiresAction` (off-session omits 3DS).

### `ReconcileOps::fetch_transaction_status`
`GET /v2/payments/{id}` → same status table as above onto
`RemoteTransactionStatus` (`disputed` → `Succeeded`); HTTP 404 → `Unknown`.
This + the generic `reconciliation_worker` **replaces the fork's bespoke
`stancer_payment_polling_worker.rs` entirely.**

### `RefundOps::refund`
`POST /v2/refunds/` — `{payment: <paym_…>, amount?}` (omit `amount` = full).
`RefundStatus` → outcome: `refunded` → `Succeeded`;
`to_refund` / `refund_sent` / `awaiting_approval` → `Pending`;
`not_honored` / `payment_canceled` / `failed` → `Failed`.
**No idempotency field on `RefundCreate`** — see §7 (double-refund risk).

### `RefundOps::fetch_refund`
`Unsupported`. Its contract purpose is resolving amount-less refund
*webhooks*; Stancer has no webhooks, and our own `refund()` returns amounts
inline. (Consequence: dashboard-initiated refunds are invisible to us — §7.)

### `WebhookOps` (all of it)
`register_webhook` / `unregister_webhook` / `sync_webhook_events` →
`Unsupported`. `verify_signature` → always
`Err(ConnectorError::SignatureVerification)` (no legitimate webhook exists);
`parse_event` → `Err(PayloadDecode)`. In practice unreachable: the router's
`webhook_secret()` `Some(_)` fallback already bails `ProviderNotSupported`
for a Stancer connector — **no router arm and no event_handler arm needed**.

## 5. Webhook map

None. **Polling-primary provider**: settlement, failure, and cancellation all
surface through `ReconcileOps::fetch_transaction_status` via the generic
reconciliation worker; mandate completion surfaces through the return-URL
redirect (§6). Do not invent a signature scheme.

Latency note: the reconcile worker only picks up transactions Pending for
`PENDING_AGE_THRESHOLD` (10 min) and sweeps every 60s. With no webhook to
short-circuit, a Stancer card payment will typically show as settled 10–12
minutes after capture. Acceptable for recurring; see §7 for the option of a
provider-aware threshold.

## 6. Flows

### Recurring (save card, charge later) — the meteroid core flow
1. `create_customer` → `cust_…` (once per connection).
2. Portal `SetupIntent` rpc → `initiate_mandate_setup` →
   `HostedRedirect { pi_…, url }` (mapped into the existing `SetupIntent`
   proto: `intent_secret` carries the URL, same as GC — no proto change).
3. Frontend hosted-redirect branch (same component family as GoCardless)
   sends the browser to `url` (full-page redirect, v1).
4. Customer enters card + completes 3DS on `payment.stancer.com`.
5. Hosted page auto-redirects (~3s) to our
   `/v1/portal/stancer/return?connection=…&intent=pi_…&dest=…`.
6. **Return handler is ON the money path** (the one structural difference
   from GoCardless, whose completion is webhook-driven): it calls a new
   `Services::complete_stancer_setup(connection_id, intent_id)` mirroring
   `gocardless_return.rs` — `complete_mandate_setup` → ownership check
   (intent metadata must name this connection + customer; endpoint is
   unauthenticated) → `upsert_payment_method` + set as default → then, if the
   intent metadata names `meteroid.invoice_id` or
   `meteroid.checkout_session_id`, trigger the corresponding charge /
   checkout activation via `charge_off_session` (card charges are instant to
   Pending, unlike DD). Redirect back to `dest` with a status marker.
7. Renewal charges: `charge_off_session` → usually
   `ChargeOutcome::Pending` (`to_capture`) → generic reconcile worker polls
   `fetch_transaction_status` until `captured` → `Succeeded`.

### One-off / hosted checkout
Same rails: v1 keeps the intent at `amount: 0` (card save only) and performs
the first payment as a **server-initiated** `charge_off_session` inside step 6
(fail-closed: if the customer never returns, no money has moved — the inverse
failure of GC's in-flow `payment_request`, where the webhook backstops a lost
redirect; Stancer has no such backstop). The local Pending checkout
transaction records the resulting `paym_…` as `provider_transaction_id`;
reconcile settles it. In-flow capture (`amount > 0, capture: true`, first
payment = `intent.payment`) is possible but rejected for v1 — see §7(c).

## 7. Open questions / gaps / risks

**(a) Contract fit — no blocker.** All flows land on existing variants.
Wrinkles, all cosmetic: `webhook_replay_tolerance_secs` must be > 0 by
contract even though moot (use 3600 + comment); `fetch_refund` is
`Unsupported` while `supports_refunds = true` (allowed — GC has the inverse
split); `RequiresAction` is never produced.

**(b) Refund idempotency — needs a decision.** `RefundCreate` has no
idempotency field. A `Transport`-retried `refund()` can double-refund.
Mitigation options: (1) before creating, `GET /v2/payments/{id}/refunds` and
skip if a matching-amount refund already exists (racy but narrow); (2) treat
refund errors as non-retryable at the call site. Recommend (1)+(2).

**(c) Lost-return exposure — needs a decision (v1 answer proposed above).**
With no webhook, a customer who pays on the hosted page and closes the
browser before the redirect leaves the flow incomplete. With the v1 0-amount
design this is fail-closed (card saved at Stancer, nothing charged, no local
state — customer retries). If we ever switch checkout to in-flow capture,
money could move with no local record; that would require persisting the
intent id at initiation plus a pending-intent sweeper (poll
`GET /v2/payment_intents/{id}` for open intents) before it's safe.

**(d) $0-authorization on real card networks — unverified end-to-end.** The
API accepts `amount: 0` (schema minimum 0; fork verified the create call),
but whether every acquirer/network completes a genuine 0-amount verification
is unproven without a live card test. Documented fallback: nominal amount
with `capture: false`, then cancel the resulting payment.

**(e) 3DS liability.** Setup runs 3DS (`threeds: required` on the intent);
subsequent off-session charges omit `auth` → no 3DS, merchant liability on
those charges (standard MIT posture; Stancer exposes no MIT/CIT exemption
flags in the spec). Accept and document.

**(f) iframe vs redirect.** The hosted page supports both; in iframe mode it
postMessages success to the parent and still needs `return_url` to allow
framing. v1 = full-page redirect (matches `MandateSetupMode::HostedRedirect`,
reuses the GC frontend branch, avoids resurrecting the fork's
`GetSetupIntentStatus` polling rpc). The fork's iframe + poll UI
(`StancerHostedCardForm.tsx`, `GetSetupIntentStatus`) is **dropped**; iframe
+ postMessage is a possible later UX upgrade (would not touch the adapter).

**(g) `.card` timing at return.** The redirect may beat the intent's own
`.card` update. The return service should retry `complete_mandate_setup` a
few times (e.g. 3 × 2s) before surfacing "processing"; there is no webhook
fallback behind it.

**(h) Settlement latency.** 10-minute reconcile threshold (see §5) is the
only settlement signal. Option if product wants faster: per-provider (or
capabilities-driven: `!supports webhooks`) age threshold in the reconcile
sweep. Not required for v1.

**(i) Currency.** Fork hardcoded `"eur"` on the save-card intent. Stancer
supports eur, aud, cad, chf, dkk, gbp, nok, pln, sek, usd. The port should
pass the real currency (customer/invoice currency) on intents and charges;
validate against the supported set and fail with a clear `Charge`/
`MandateSetup` error otherwise.

**(j) Customer-create conflict.** `external_id` is unique; a retried create
409s/422s. Handle by treating the conflict as "already exists" (lookup), not
as a hard failure.

**(k) Dashboard-initiated refunds/disputes are invisible** (no webhook, and
reconcile only watches Pending transactions). Known gap, shared shape with
"GC refunds before #1242". Future option: periodic
`GET /v2/refunds/?…` / `GET /v2/disputes/` sweep. Out of v1 scope.

## 8. Port plan (file-by-file)

Legend: **REUSE** = fork file lands ~verbatim; **ADAPT** = fork file with
mechanical adjustments; **REWRITE** = new code informed by the fork;
**NEW** = no fork counterpart; **DROP** = fork file/change intentionally not
ported.

### Stage A — enum threading (SEQUENTIAL, compiler-driven)
1. `migrations/diesel/<new-date>_stancer_payment_provider/{up,down}.sql` —
   **REUSE** fork's (`ALTER TYPE "ConnectorProviderEnum" ADD VALUE IF NOT
   EXISTS 'STANCER';`), new dated directory.
2. `crates/diesel-models/src/enums.rs` — **REUSE** (`Stancer` variant +
   `as_meta_key() => "stancer"`).
3. `crates/meteroid-store/src/domain/enums.rs` — **REUSE** (append after
   `Gocardless`; o2o names must match).
4. `proto/api/connectors/v1/models.proto` — **ADAPT**: `STANCER = 4`
   (**not** the fork's `= 3` — `GOCARDLESS` took 3 on main); append
   `StancerConnector { alias, api_secret_key }`.
   `proto/api/connectors/v1/connectors.proto` — **REUSE** (`ConnectStancer`
   rpc + messages). Regenerate Rust + TS
   (`pnpm --prefix modules/web --filter @md/web generate:proto`).
5. `src/api/connectors/mapping.rs` — **REUSE** fork arms (both directions +
   `ProviderData::Stancer(_) => None` + `stancer_data_to_domain`).
6. Compiler sweep: `factory.rs` match (add in Stage C), any other
   non-exhaustive matches the build flags.

### Stage B — domain config + client crate (parallel with each other, after A)
7. `crates/meteroid-store/src/domain/connectors.rs` — **REUSE**
   (`StancerPublicData {}`, `StancerSensitiveData { api_secret_key }`,
   both enum variants).
8. `crates/stancer-client/` (9 files + Cargo.toml; workspace `Cargo.toml`,
   `modules/meteroid/Cargo.toml`, `meteroid-store/Cargo.toml` entries) —
   **REUSE ~verbatim**; **ADD** `update_payment_intent`
   (`PATCH /v2/payment_intents/{id}`), `refunds.rs` (`create_refund`,
   `get_refund`, `RefundStatus`), optionally `list_payment_refunds`.

### Stage C — the adapter (after B; the core REWRITE)
9. `crates/meteroid-store/src/adapters/payment/stancer.rs` — **REWRITE**
   onto the sub-trait family per §4 (port status maps + live-verified notes
   from the fork's `payment_service_providers.rs` impl and
   `stancer_payment_to_domain`). `OnceLock<StancerClient>` singleton (one —
   no sandbox/live split). Unit tests mirroring `gocardless.rs`'s
   (status→outcome tables, capability pin, completion metadata recovery).
10. `adapters/payment/mod.rs` + `factory.rs` — **NEW** one-line arms.
11. Contract: capability-consistency + `register_webhook is Unsupported`
    tests alongside `contract.rs`'s GC tests — **NEW**.

### Stage D — completion path (parallel with C once B lands)
12. `crates/meteroid-store/src/services/payment/stancer_return.rs` —
    **NEW** (mirror `gocardless_return.rs`: ownership check on intent
    metadata, upsert method, set default) **plus** the post-completion
    charge/checkout-activation step (§6 step 6) that GC does in its webhook
    handler. `services/payment/mod.rs` + `services/edge.rs` passthrough —
    **ADAPT**.
13. `src/api_rest/stancer/{mod.rs, return_handler.rs}` — **NEW** (mirror
    `api_rest/gocardless/`, route
    `/v1/portal/stancer/return`; unlike GC's, calls
    `complete_stancer_setup` before redirecting; same `safe_dest`
    open-redirect defense + error-code sanitizing). Register in
    `api_rest/mod.rs`.
14. Webhook layer: **no changes** (router's `Some(_)` fallback already
    rejects Stancer; no event_handler arm — Stancer emits no events).

### Stage E — connect surface (parallel with C/D)
15. `crates/meteroid-store/src/repositories/connectors.rs` —
    **REUSE** fork's `connect_stancer`.
16. `src/api/connectors/service.rs` — **REUSE** fork's `connect_stancer`
    handler (ping-validates the key before persisting).

### Stage F — frontend (parallel with C/D/E after protos regenerate)
17. `features/settings/integrations/schemas.ts` — **REUSE**
    (`stancerIntegrationSchema`: alias + apiSecretKey, zod).
18. `features/settings/integrations/StancerIntegration.tsx` — **REUSE**
    (align with current modal/route conventions next to
    `GoCardlessIntegration.tsx`).
19. `features/settings/tabs/IntegrationsTab.tsx` — **ADAPT** fork's card +
    route onto main's current file.
20. `features/settings/tabs/PaymentsTab.tsx` — **NEW** one line:
    `PROVIDER_CAPABILITIES[STANCER] = { card: true, directDebit: false }`.
21. `features/customers/modals/ManageConnectionsModal.tsx` — **NEW**
    `getProviderName` arm.
22. `features/checkout/PaymentPanel.tsx` (+ `CheckoutFlow.tsx`,
    `InvoicePaymentFlow.tsx`, `pages/portal/customer/AddPaymentMethodDialog.tsx`)
    — **REWRITE** onto the existing hosted-redirect branch (reuse the GC
    redirect card component; `intentSecret` = hosted page URL); generalize
    `features/checkout/utils/gocardlessReturn.ts` or add a sibling for the
    `stancer_status` return markers.
23. **DROP**: `StancerHostedCardForm.tsx` (iframe + poll),
    fork's `EmbedHost.tsx` / `AddPaymentMethodDialog.tsx` iframe diffs.

### Dropped fork backend surface (superseded by the new architecture)
- `src/workers/misc/stancer_payment_polling_worker.rs` + `workers/mod.rs`
  wiring + `errors.rs::StancerPaymentPolling` — **DROP** (generic
  `reconciliation_worker` + `ReconcileOps`).
- `diesel-models/src/query/payment_transactions.rs::list_pending_payment_tx_by_provider`
  + repository plumbing — **DROP** (generic `list_pending_with_provider_id`).
- `GetSetupIntentStatus` rpc (`portal/shared/v1/shared.proto`), portal
  service method, `Services::get_setup_intent_status`, and
  `SetupIntentStatus` domain type — **DROP** (redirect return replaces
  polling-from-the-browser).
- All fork changes to `payment_service_providers.rs` — file no longer exists.

### Tests
24. `tests/integration/…` (fork's `payment_methods_config.rs`, `data/ids.rs`,
    `data/payment.rs`, `harness/payments.rs`) — **ADAPT** to the new
    connector seams; add adapter unit tests (Stage C) for status maps and
    completion ownership checks.

### Parallelization summary
- **Sequential spine**: Stage A (enum in 5 layers, compiler-driven) → then B.
- **Parallel after B**: Stage C (adapter) ∥ Stage D (return path) ∥ Stage E
  (connect surface) ∥ Stage F (frontend, once protos are regenerated).
- Final integration pass: factory/mod registration + tests touch shared
  files — land last.
