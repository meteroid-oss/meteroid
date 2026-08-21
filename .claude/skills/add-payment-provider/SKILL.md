---
name: add-payment-provider
description: Add a new payment provider (Adyen, Mollie, Stancer, Braintree, …) to the meteroid multi-provider payment layer. Research-first — finds the official client/OpenAPI to derive request shapes, enumerates webhooks + schemas, reads the recurring (mandate/off-session) and one-off integration flows, writes a research doc, and only then implements the PaymentConnector adapter end-to-end. Use whenever asked to add/integrate/support a new payment gateway or provider.
---

# Add a payment provider

Goal: take a provider from zero to a working `PaymentConnector` in minimum time,
**without guessing API shapes or webhook payloads**. The abstraction is already
pluggable (one adapter + one enum threaded through the stack); the risk is
integrating against imagined endpoints. So: **research → document → implement**,
in that order. Do not write the adapter before the research doc exists.

The payment layer lives at
`modules/meteroid/crates/meteroid-store/src/adapters/payment/` — **note the name
collision**: the top-level `modules/adapters/` (openstack, slurm-collector) is
unrelated infra and has no payment code. Always work in the nested `meteroid-store`
one.

Key source files to reread each run (they drift — don't trust this skill's line
numbers, re-grep):
- `.../adapters/payment/connector.rs` — the `PaymentConnector` trait (sub-traits) + `ConnectorCapabilities` + `MandateSetupMode`
- `.../adapters/payment/factory.rs` — `initialize_payment_connector` dispatch match
- `.../adapters/payment/model.rs` — `ChargeRequest`, `MandateSetupInstruction`, `ChargeOutcome`, …
- `.../adapters/payment/events.rs` — the normalized webhook vocabulary you must map onto
- `.../adapters/payment/{stripe,gocardless,mock}.rs` — the two real templates + the stub template
- `.../domain/connectors.rs` — `ProviderData` / `ProviderSensitiveData` enums + per-provider config structs
- `.../domain/enums.rs` + `crates/diesel-models/src/enums.rs` — the `ConnectorProviderEnum`

---

## Phase 0 — Scope (before any research)

Pick the closest existing template by the provider's model, because it decides
`mandate_setup_mode` and half the adapter:
- **Cards + embedded SDK + self-registering webhooks** → template on `stripe.rs`
  (`MandateSetupMode::EmbeddedClientSecret`).
- **Bank debit / redirect mandate, dashboard-managed webhooks** → template on
  `gocardless.rs` (`MandateSetupMode::HostedRedirect`).
- **Hosted drop-in widget** → `MandateSetupMode::EmbeddedDropIn`.

State which template you're using and why. If the provider spans both (Adyen,
Mollie do cards *and* bank debits), pick the primary rail for the first pass and
note the rest as follow-ups — don't try to land everything at once.

---

## Phase 1 — Research (the important part)

Use `WebSearch` + `WebFetch`. **Prefer official sources**, in this priority order,
and record the URL of every source you rely on:

1. **Official OpenAPI / API reference** — the source of truth for request/response
   shapes. Search `"<provider> openapi spec"`, `"<provider> api reference"`,
   `site:docs.<provider>.com`. If they publish an OpenAPI/Swagger JSON, fetch it —
   it gives you exact field names, types, and required/optional for every request.
2. **Official client library** — search `"<provider> official rust client"`, then
   `"<provider> official <lang> sdk"` (node/python/java are fine as shape
   references even if we don't use them). Check crates.io / the provider's GitHub
   org. An official Rust crate may be usable directly (like `stripe-client` wraps);
   an official non-Rust SDK is still gold for deriving the request/response structs
   to hand-roll in an `<provider>-client` crate. **Community/unofficial crates:
   reference only, never depend on without flagging it to the user.**
3. **Webhook / notifications reference** — search
   `"<provider> webhooks event types"`, `"<provider> notification reference"`.
   Enumerate: the full **event/notification type list**, the **payload schema**,
   and the **signature verification scheme** (HMAC header name, algorithm, replay
   window). This maps directly onto `WebhookOps::{verify_signature, parse_event}`
   and `events.rs::NormalizedEventKind`.
4. **Integration flow guides** — read the actual step-by-step docs for **both**:
   - **Recurring / off-session** (tokenization / mandate → store token → charge
     off-session later). This is the meteroid core flow. Note whether mandate setup
     is embedded-secret, hosted-redirect, or drop-in — it must match your Phase 0
     `mandate_setup_mode`.
   - **One-off / on-session** payment (if the provider distinguishes it).
   Capture: how a customer is created, how a payment method/mandate is set up and
   confirmed, how an off-session charge is made, how refunds work, and how each step
   is confirmed (sync response vs async webhook → sets `asynchronous_settlement`).

For each provider concept, resolve it to our contract before writing code:

| Provider concept | Maps to |
|---|---|
| create customer / shopper | `CustomerOps::create_customer` |
| tokenize / setup mandate / setup intent | `MandateOps::initiate_mandate_setup` → a `MandateSetupInstruction` variant |
| redirect return / server-side confirm | `MandateOps::complete_mandate_setup` |
| fetch stored method / mandate detail | `MandateOps::fetch_payment_method` |
| off-session charge with stored token | `PaymentOps::charge_off_session` → `ChargeOutcome` |
| refund | `RefundOps::refund` |
| poll payment status | `ReconcileOps::fetch_transaction_status` |
| each webhook/notification type | a `NormalizedEventKind` (or intentionally dropped → log) |
| card / bank-debit / 3DS / disputes / partial-refund support | `ConnectorCapabilities` bits |
| sync-confirm vs webhook-confirm | `asynchronous_settlement` |
| can we create webhook endpoints via API? | `supports_self_webhook_registration` |
| signature max age | `webhook_replay_tolerance_secs` |

If a provider capability has **no** mapping in our contract, stop and raise it with
the user before inventing one — it may need a new `NormalizedEventKind`, a new
`MandateSetupInstruction` variant, or a new capability bit.

---

## Phase 2 — Document (the deliverable that de-risks implementation)

Write `.../adapters/payment/research/<provider>.md` containing:

1. **Sources** — every official URL used (API ref, OpenAPI, SDK repo, webhook ref,
   flow guides), so a reviewer can verify against the same docs.
2. **Auth & environments** — credential types (API key / OAuth), sandbox vs live
   base URLs, what goes in `<Provider>PublicData` vs `<Provider>SensitiveData`.
3. **Capability matrix** — the exact `ConnectorCapabilities` values you'll declare,
   with a one-line justification each (esp. `mandate_setup_mode`,
   `asynchronous_settlement`, `supports_self_webhook_registration`).
4. **Request map** — for each trait method, the concrete endpoint(s), method, key
   request fields, and success/error response shape (cite the OpenAPI/SDK).
5. **Webhook map** — table of provider event type → `NormalizedEventKind` (or
   "dropped, logged"), plus the signature scheme (header, algorithm, replay window).
6. **Recurring flow** and **one-off flow** — the step sequence, annotated with which
   trait method + which `MandateSetupInstruction`/`ChargeOutcome` variant each step
   produces.
7. **Open questions / gaps** — anything the docs didn't answer, unofficial-source
   caveats, or capabilities with no contract mapping.

**Checkpoint:** summarize the research doc to the user and confirm the capability
matrix + template choice before implementing. This is the cheapest place to catch a
wrong assumption.

---

## Phase 3 — Implement

Grounded in the research doc (no invented endpoints or payloads). Order that
compiles incrementally — the enum is threaded through 5 layers, and the compiler's
non-exhaustive-match errors are your live checklist:

1. **Enum variant, all layers:**
   - DB migration `modules/meteroid/migrations/diesel/<date>_<name>/up.sql`:
     `ALTER TYPE "ConnectorProviderEnum" ADD VALUE IF NOT EXISTS 'ADYEN';`
     (Postgres can't drop enum values — say so in `down.sql`).
   - `crates/diesel-models/src/enums.rs` (variant + `as_meta_key()` arm).
   - `crates/meteroid-store/src/domain/enums.rs` (o2o-mapped variant — names must match).
   - `proto/api/connectors/v1/models.proto` (append, never renumber) → regenerate Rust + TS.
   - `src/api/connectors/mapping.rs` (both proto↔domain arms).
2. `ProviderData` / `ProviderSensitiveData` variants + config structs in
   `domain/connectors.rs` (public vs encrypted split per the research doc;
   sandbox-default any live toggle).
3. `<provider>-client` crate if hand-rolling (mirror `gocardless-client` shape), or
   wire the official crate. Hold the HTTP client in a `OnceLock` inside the adapter.
4. `<provider>.rs` adapter — implement every `PaymentConnector` sub-trait. Start
   from a stub returning `ConnectorError::Unsupported` everywhere; fill method by
   method against the research doc's request map. Register in `mod.rs` + `factory.rs`.
5. `webhook_secret()` arm in `api_rest/webhooks/router.rs`; `match &connector.data`
   arm in `api_rest/webhooks/event_handler.rs`. (Inbound route + event dispatch are
   already provider-agnostic — no route to add.)
6. gRPC `Connect<Provider>` (message in `models.proto` + rpc in `connectors.proto` +
   handler in `api/connectors/service.rs`); return-handler route under
   `api_rest/<provider>/` if hosted-redirect (mirror `api_rest/gocardless/`).
7. Frontend (`web-app/`): integration card in `settings/tabs/IntegrationsTab.tsx`,
   route in `settings/settingsTabs.tsx` + a modal in `settings/integrations/`,
   `PROVIDER_CAPABILITIES` in `settings/tabs/PaymentsTab.tsx`, `getProviderName()` in
   `customers/modals/ManageConnectionsModal.tsx`, and a `checkout/PaymentPanel.tsx`
   branch (reuse the redirect branch for hosted-redirect).

Rules the abstraction depends on (repeated because they're easy to violate):
- Unsupported op → `ConnectorError::Unsupported`, **never `panic!`**.
- Thread the `IdempotencyKey` through every provider call.
- `Ok(ChargeOutcome::Failed)` = terminal refusal; `Err(Transport)` = unknown/retryable only.
- No provider-specific type leaks past the adapter — webhooks parse into
  `NormalizedWebhookEvent`.

---

## Phase 4 — Verify

- Add a contract test calling `run_contract(&impl_, &connector)` in `contract.rs`
  (or the adapter module) — proves idempotency threading, capability
  self-consistency, and that unsupported ops error instead of panic.
- Add a sandbox test alongside `gocardless-client/tests/` if the provider has a
  usable sandbox.
- `cargo build` and re-grep the existing providers (`grep -rin gocardless`) — every
  compiler-flagged non-exhaustive match arm is a site you must handle.
- Run the billing-reviewer agent over the money-handling paths before finishing.

---

## Guardrails

- **Research before code.** If asked to "just add Adyen fast", still produce the
  research doc first — it *is* the fast path; guessing endpoints costs more later.
- **Official sources win.** Cite them. Flag any reliance on unofficial clients.
- **One rail first** for multi-rail providers; land it, then iterate.
- Sandbox-default any live/sandbox toggle (see `GocardlessPublicData::default_environment`)
  so a malformed config never routes real money to live.
