# Payment adapters — testing & 3DS/SCA flow

## Test strategy (recommended)

Three tiers, increasing cost/coverage:

1. **Unit tests (in-tree, no network).** Signature verification, event
   parse/normalize, charge-outcome mapping, `PaymentNextAction` storage/redaction.
   These live in each adapter's `#[cfg(test)]` module. Always run in CI.

2. **Mock-driven flow tests (no real provider).** The `MockConnector`
   (`mock.rs`) is a full test double:
   - `MockPublicData.charge_behavior` forces `succeeded` / `pending` (async) /
     `requires_action` (3DS) / `failed`.
   - `capabilities()` advertises async + 3DS so flow code exercises those paths.
   - `parse_event` accepts a small JSON envelope (`MockWebhookEvent`:
     `{"id","kind","transaction_id",...}`) so the webhook→settlement path can be
     driven deterministically — `kind` ∈ {`payment_succeeded`, `payment_failed`,
     `payment_pending`, `payment_requires_action`, `payment_method_attached`,
     `payment_method_updated`}.

   This drives end-to-end state transitions against a real Postgres test DB
   (testcontainers), through the *production* webhook dispatcher
   (`api_rest::webhooks::event_handler::handle_normalized_event`) — the same
   path the HTTP route uses, minus signature verification. Shipped in
   `tests/integration/subscription/payment_webhook_settlement.rs`:
   - charge `pending` → webhook `payment_succeeded` → transaction `Settled`,
     invoice paid;
   - charge `requires_action` → `next_action` persisted on the pending
     transaction → webhook `payment_succeeded` → `Settled` + `next_action`
     cleared;
   - charge `pending` → webhook `payment_failed` → transaction `Failed`,
     invoice stays unpaid.

   `TestEnv::set_mock_charge_behavior(..)` configures the mock; assertions read
   transaction status via the store and the raw `next_action` JSONB column
   (the domain `PaymentTransaction.next_action` is intentionally ghosted — it's
   transient on the charge response and never re-hydrated from the row).

3. **Stripe schema-conformance tests against `stripe-mock` (no credentials).**
   `stripe-mock` serves Stripe's real OpenAPI surface: it validates each request
   against the published schema and answers with schema-valid fixtures. The
   suite in `stripe-client/tests/stripe_mock.rs` proves our request
   serialization (`serde_qs` form/bracket encoding, the flattened
   `mandate_data[...]` keys, repeated `payment_method_types[]`) and response
   deserialization match Stripe's contract. Gated on `STRIPE_MOCK_URL`:

   ```bash
   stripe-mock -http-port 12111 &
   STRIPE_MOCK_URL=http://127.0.0.1:12111/ cargo test -p stripe-client --test stripe_mock
   ```

   (This is what caught the missing `Content-Type: application/x-www-form-urlencoded`
   header on form POSTs — real Stripe is lenient, the spec is not.)

4. **Gated provider sandbox tests (creds).** Real provider sandbox integration
   tests, gated on env vars: `stripe-client/tests/stripe_sandbox.rs` on
   `STRIPE_SECRET_KEY`, and `gocardless-client/tests/gocardless_sandbox.rs` on
   `GOCARDLESS_ACCESS_TOKEN`. Both skip gracefully (green pass) when credentials
   are absent. Stripe test-mode is deterministic and worth 1–2 e2e cases (card,
   3DS via `pm_card_authenticationRequired`, a DD scheme). GoCardless sandbox
   exercises the full flow: customer creation, mandate setup via Billing Request
   + Flow, payment creation, webhook parsing.

We ship tiers 1–4. Tiers 1–2 always run in CI. Tier 3 (stripe-mock) runs when
`STRIPE_MOCK_URL` is set. Tier 4 (real provider sandbox) requires credentials
and is typically run locally or in optional pre-merge checks; both suites skip
silently in CI when env vars are absent.

## 3DS / `requires_action` (SCA) flow

A charge can come back needing customer authentication. The path differs by
whether the customer is present.

### On-session (checkout, paying an invoice from the portal, slot upgrade)
Resolved inline, no email/admin involvement:
1. Backend charges with `on_session: true`; Stripe returns `requires_action`
   plus a `client_secret`.
2. The charge response carries a `PaymentNextAction::UseSdk { intent_id,
   publishable_key, client_secret }` (the secret is **transient**, never stored).
3. The portal calls `stripe.handleNextAction(client_secret)` → 3DS modal →
   customer authenticates.
4. `payment_intent.succeeded` webhook → transaction settled, `next_action` cleared.

### Off-session (recurring auto-billing — no customer present)
Can't prompt; this becomes a dunning step:
1. Backend charges with `on_session: false`; Stripe signals `requires_action`
   (synchronously or via the `payment_intent.requires_action` webhook).
2. The transaction stays `Pending` with a stored `next_action` (intent id +
   publishable key, **no secret**).
3. The customer is brought back on-session via an emailed portal link; the
   portal re-fetches a fresh `client_secret` from the provider and completes
   `handleNextAction`.

`PaymentStatusEnum` has no `RequiresAction` variant by design: "awaiting
authentication" = `Pending` + `next_action IS NOT NULL`.

### Secret handling
The PaymentIntent `client_secret` is a capability for that one intent. It is
**never persisted** (serde `#[skip]` on the field keeps it out of the DB;
`SecretString`'s `Debug` impl redacts it from logs) and only travels transiently
in the on-session response or after an on-demand re-fetch.

## Card `updated` / `expiring`
- `payment_method.updated` / `automatically_updated` → refresh stored
  brand/last4/expiry (`update_payment_method_card_details`).
- `payment_method.expiring` → logged for the notification hook (customer should
  update the card); no DB change.

## Implemented vs remaining
- Done: state machine + persistence, Stripe adapter capturing the real
  `client_secret`, webhook handling (`requires_action`, method
  `updated`/`expiring`), mock e2e support, the off/on-session distinction
  threaded through all charge flows. On-session 3DS is surfaced for **every**
  on-session flow — invoice payment, all checkout types (self-serve,
  subscription activation, plan change, addon) and slot upgrade — via
  `next_action` on their confirm responses; the portal (`completeNextAction`)
  calls Stripe.js `handleNextAction`. Slot upgrade and subscription activation
  defer their effect to settlement: a pending slot transaction activates on the
  payment webhook (`activate_pending_slot_transactions`), and activation
  completes via `on_checkout_payment_settled`.
- Remaining: the off-session customer email (dunning) is a hook only for now.
  After pulling the proto change, run `pnpm generate:proto` to regenerate the TS
  bindings.

## Stripe API version

Bumped to `2026-04-22.dahlia` (`stripe-client::client::API_VERSION`). Schema
conformance against that version is only checked when `STRIPE_MOCK_URL` (tier
3 above) is set. Tier 4 (real sandbox) validates live, but skips in CI by
design (requires `STRIPE_SECRET_KEY` / `GOCARDLESS_ACCESS_TOKEN`).
Recommend wiring a pre-merge job that starts `stripe-mock` and runs the tier-3
suite on every PR touching `stripe-client`/`stripe.rs`, so a version bump that
breaks request/response shape fails before merge instead of in production.
