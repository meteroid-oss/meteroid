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

   Use this to assert end-to-end state transitions (charge Pending →
   webhook succeeded → Settled; charge requires_action → next_action persisted →
   webhook succeeded → Settled + next_action cleared) against the test DB,
   without any Stripe/GoCardless credentials.

3. **Gated provider sandbox tests (creds, nightly/manual).** Mirror hyperswitch:
   read credentials from an env var, `#[ignore]` by default, run only when
   present. Stripe test-mode is deterministic and worth 1–2 e2e cases (card,
   3DS via `pm_card_authenticationRequired`, a DD scheme). GoCardless sandbox is
   harder: mandate setup needs the hosted Billing Request Flow (browser
   consent), so keep mandate `active` as a fixture test and use the payment
   scenario simulator only for payment status transitions.

We currently ship tiers 1 and 2. Tier 3 is documented but not wired (needs a
secrets channel in CI).

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
**never persisted** (`PaymentNextAction::for_storage` strips it; `Debug` redacts
it) and only travels transiently in the on-session response or after an
on-demand re-fetch.

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
