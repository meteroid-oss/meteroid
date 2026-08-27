---
name: add-tax-provider
description: Add a new external tax provider (Kintsugi, Avalara, TaxJar, Stripe Tax, …) to the meteroid tax layer. Research-first — finds the official client/OpenAPI to derive the calculation + nexus + validation request shapes, maps them onto the TaxEngine trait, writes a research doc, and only then implements the engine and threads a Tax-typed connector through the stack. Use whenever asked to add/integrate/support an external tax calculation provider.
---

# Add a tax provider

Goal: take an external tax provider from zero to a working `TaxEngine`
implementation, wired as a per-invoicing-entity `Tax` connector, **without
guessing API shapes or nexus/validation semantics**. The abstraction is already
pluggable (one engine impl behind one extension point, plus a connector enum
threaded through the stack); the risk is integrating against imagined endpoints.
So: **research → document → implement**, in that order. Do not write the engine
before the research doc exists.

The tax layer lives in two places:
- `modules/meteroid/crates/meteroid-tax/` — the `TaxEngine` trait and the two
  built-in engines (`MeteroidTaxEngine` = world-tax/VIES, `ManualTaxEngine`).
- `modules/meteroid/crates/meteroid-store/src/services/invoice_lines/invoice_lines.rs`
  — `build_tax_engine`, **the single extension point** that chooses the engine
  for an invoicing entity.

Key source files to reread each run (they drift — don't trust this skill's line
numbers, re-grep):
- `.../meteroid-tax/src/lib.rs` — the `TaxEngine` trait (`validate_vat_number`,
  `calculate_line_items_tax`, `calculate_customer_tax`) + `MeteroidTaxEngine` /
  `ManualTaxEngine` templates.
- `.../meteroid-tax/src/model.rs` — `Address`, `CustomerForTax` (carries
  `billing_address` and an optional `shipping_address` ship-to override),
  `LineItemForTax`, `CustomerTax`, `CalculationResult`, `TaxBreakdownItem`,
  `VatNumberExternalValidationResult` — the exact input/output types you must
  produce and consume.
- `.../meteroid-store/src/services/invoice_lines/invoice_lines.rs` —
  `build_tax_engine` (where you register the provider).
- `.../meteroid-store/src/domain/connectors.rs` — `ProviderData` /
  `ProviderSensitiveData` enums + per-provider config structs.
- `.../meteroid-store/src/domain/enums.rs` + `crates/diesel-models/src/enums.rs`
  — the `ConnectorProviderEnum` and `ConnectorTypeEnum` (`Tax`).
- `.../domain/invoicing_entities.rs` — `tax_provider_id` (the connector the
  entity points at) and `default_tax_category_id`.

How a tax provider hangs together: an invoicing entity may point
`tax_provider_id` at a `Tax`-typed `connector` row (encrypted credentials, like
any payment/CRM connector). `build_tax_engine` sees that id, loads the connector,
matches on its `provider`, and returns your `TaxEngine`. When `tax_provider_id`
is NULL the built-in resolver (`tax_resolver`) is used instead. Tax categories
(`tax_category`, `product.tax_category_id`, `invoicing_entity.default_tax_category_id`)
are provider-agnostic classifications carried on each `LineItemForTax`; an
external provider maps them to its own product-tax codes.

---

## Phase 0 — Scope (before any research)

An external tax provider replaces our *calculation* (and possibly *validation*)
with theirs. Decide up front:
- **Which `TaxEngine` methods the provider actually backs.** Most back
  `calculate_line_items_tax` (line-level rates by jurisdiction + product tax
  code). Some also back `validate_vat_number` / registration lookups; if not,
  delegate that method to the existing VIES path rather than stubbing it.
- **Nexus / registration model.** Does the provider decide taxability itself
  (you send addresses + amounts + product codes, it returns rates), or do you
  configure nexus out-of-band in their dashboard? This decides how much config
  lives in `<Provider>PublicData`.
- **Addresses you'll actually have.** Ship-from is the invoicing entity's
  address (passed in full). Ship-to is `CustomerForTax.shipping_address` when the
  customer set a distinct one, else `billing_address` — resolve
  `shipping_address` then fall back to `billing_address`. Both are our `Address`
  (country, region ISO-3166-2, city, postal, line1) — no line2. If the provider
  needs rooftop accuracy the customer hasn't supplied, that's a data gap to
  surface, not something to invent.
- **What the trait does NOT carry today** — decide whether the provider needs
  any of these, and raise it before implementing rather than stubbing:
  - a stable **customer code** (providers that store exemption certificates
    server-side, e.g. Avalara CertCapture, key on it) — not passed;
  - an **exemption certificate / entity-use code** — only a `tax_exempt` bool
    reaches the engine, not the reason/certificate;
  - a **commit / void document lifecycle**. `TaxEngine` is calculate-only and
    passes no invoice-level document id (only a per-line `line_id`). A *filing*
    provider (Kintsugi, Avalara) must record the committed transaction on
    finalize and void it on void so its returns match the invoices actually
    issued — that needs new trait method(s) + a stable document id threaded from
    the invoice, which is an architectural change to agree with the user first.
- **Category mapping.** How our `tax_category` keys map to the provider's product
  tax codes. Note it now; it becomes a lookup table in the engine.
- **Sync vs async.** Tax calc is request/response for every provider worth
  integrating; if a provider only does async/batch, stop and raise it — the
  invoice-line flow calls `calculate_line_items_tax` synchronously.

State which built-in engine you're templating on (`MeteroidTaxEngine` is the
closest — it does a real external call shape) and why.

---

## Phase 1 — Research (the important part)

Use `WebSearch` + `WebFetch`. **Prefer official sources**, in this priority
order, and record the URL of every source you rely on:

1. **Official OpenAPI / API reference** — the source of truth for
   request/response shapes. Search `"<provider> api reference"`,
   `"<provider> tax calculation api"`, `site:docs.<provider>.com`. If they
   publish OpenAPI/Swagger JSON, fetch it — exact field names, types, and
   required/optional for the calculation, address-validation, and
   registration/nexus endpoints.
2. **Official client library** — search `"<provider> official rust client"`,
   then `"<provider> official <lang> sdk"` (node/python/java are fine as shape
   references). An official Rust crate may be usable directly; an official
   non-Rust SDK is still gold for deriving the request/response structs to
   hand-roll in a `<provider>-client` crate. **Community/unofficial crates:
   reference only, never depend on without flagging it to the user.**
3. **Tax calculation semantics** — read the actual calculation docs. Capture:
   how a line item is sent (amount, currency, product tax code, ship-from /
   ship-to addresses, customer tax id / exemption), whether tax is **inclusive
   or exclusive**, how **multiple jurisdictions** (state + county + city, or
   compound EU rates) come back, rounding rules, and how **exemptions** and
   **reverse charge** are signalled. This maps onto `CalculationResult` /
   `TaxBreakdownItem` and `CustomerTax`.
4. **VAT / registration validation** (only if the provider backs it) — the
   endpoint, what it returns, and how it distinguishes valid / invalid /
   service-unavailable. Maps onto `VatNumberExternalValidationResult`.

For each provider concept, resolve it to our contract before writing code:

| Provider concept | Maps to |
|---|---|
| ship-from / origin address | `invoicing_entity_address` (full `Address`) |
| ship-to / destination address | `CustomerForTax.shipping_address` ?? `billing_address` |
| calculate tax for a set of lines | `TaxEngine::calculate_line_items_tax` → `CalculationResult` |
| per-line rate + jurisdiction breakdown | `TaxBreakdownItem` (one per rate/jurisdiction) |
| single-amount / customer-level rate | `TaxEngine::calculate_customer_tax` → `CustomerTax` |
| exemption / reverse charge / no-nexus | `CustomerTax::{Exempt, NoTax}` |
| validate a VAT / tax registration id | `TaxEngine::validate_vat_number` → `VatNumberExternalValidationResult` |
| our `tax_category` key → provider tax code | a lookup in the engine (document the table) |
| API key / account id | `<Provider>SensitiveData` / `<Provider>PublicData` |

Precision rule (project-wide): **never assume 2-decimal currency.** Amounts in
`LineItemForTax` are integer minor units for the line's currency; convert with
the currency's precision, never a hardcoded `/100`. Cite how the provider
expects amounts (minor units vs decimal string) in the doc.

If a provider capability has **no** mapping in our contract, stop and raise it
with the user before inventing one — it may need a new `CustomerTax` variant, a
new field on `TaxBreakdownItem`, or a change to `LineItemForTax`.

---

## Phase 2 — Document (the deliverable that de-risks implementation)

Write `.../meteroid-tax/research/<provider>.md` containing:

1. **Sources** — every official URL used (API ref, OpenAPI, SDK repo, calc docs,
   validation docs), so a reviewer can verify against the same docs.
2. **Auth & environments** — credential types (API key / OAuth), sandbox vs live
   base URLs, what goes in `<Provider>PublicData` vs `<Provider>SensitiveData`.
3. **Calculation request map** — the concrete endpoint(s), method, exact request
   fields for a multi-line calculation (addresses, amounts + currency handling,
   product tax codes, customer tax id/exemption), and the success/error response
   shape (cite the OpenAPI/SDK).
4. **Category map** — table of our built-in `tax_category` keys → the provider's
   product tax codes, plus the fallback for the entity's `default_tax_category_id`.
5. **Response → contract map** — how the response rows become `TaxBreakdownItem`s
   and the per-line `CustomerTax`, incl. inclusive/exclusive handling, multiple
   jurisdictions, rounding, and exemption/reverse-charge signalling.
6. **Validation map** (if backed) — the endpoint and its result → each
   `VatNumberExternalValidationResult` variant; otherwise state "delegated to VIES".
7. **Open questions / gaps** — anything the docs didn't answer, unofficial-source
   caveats, or capabilities with no contract mapping.

**Checkpoint:** summarize the research doc to the user and confirm the category
map + which `TaxEngine` methods the provider backs before implementing. This is
the cheapest place to catch a wrong assumption.

---

## Phase 3 — Implement

Grounded in the research doc (no invented endpoints or payloads). Order that
compiles incrementally — the enum is threaded through several layers, and the
compiler's non-exhaustive-match errors are your live checklist:

1. **Provider enum variant, all layers** (the `Tax` connector type already
   exists — do **not** re-add it):
   - DB migration `modules/meteroid/migrations/diesel/<date>_<name>/up.sql`:
     `ALTER TYPE "ConnectorProviderEnum" ADD VALUE IF NOT EXISTS 'KINTSUGI';`
     (Postgres can't drop enum values — say so in `down.sql`).
   - `crates/diesel-models/src/enums.rs` (variant + `as_meta_key()` arm).
   - `crates/meteroid-store/src/domain/enums.rs` (o2o-mapped variant — names must match).
   - `proto/api/connectors/v1/models.proto` `ConnectorProviderEnum` (append the
     next number, never renumber) → regenerate Rust + TS
     (`pnpm --prefix modules/web --filter @md/web generate:proto`).
   - `src/api/connectors/mapping.rs` (`domain → server` arm) and
     `src/api/customers/mapping.rs` (a tax provider is not a customer
     connection: return `None`, like Mock). In `adapters/payment/factory.rs` a
     tax provider is not a payment provider — add it to the `None` /
     `ConnectorError::Unsupported` arms.
2. `ProviderData` / `ProviderSensitiveData` variants + config structs in
   `domain/connectors.rs` (public vs encrypted split per the research doc: API
   key is sensitive; account id / environment toggle is public; sandbox-default
   any live toggle).
3. `<provider>-client` crate if hand-rolling (mirror `gocardless-client` shape),
   or wire the official crate. Hold the HTTP client in a `OnceLock`.
4. **The engine**: implement `meteroid_tax::TaxEngine` for a `<Provider>Engine`
   in `meteroid-tax` (new module `.../meteroid-tax/src/<provider>.rs`). Start
   from a stub returning `TaxEngineError` everywhere; fill method by method
   against the research doc's request map. Delegate `validate_vat_number` to the
   existing `vies` path if the provider doesn't back it. Convert amounts with the
   line currency's precision — never `/100`.
5. **Register it in the extension point.** In `build_tax_engine`
   (`invoice_lines.rs`), replace the "no external tax engine is registered" error
   branch: when `invoicing_entity.tax_provider_id` is set, load that `Tax`
   connector (via the store — this makes `build_tax_engine` async; thread the
   `PgConn` and update its callers), decrypt its config, `match` on
   `connector.provider`, and return `Box::new(<Provider>Engine::new(cfg)?)`.
6. **Connector configuration surface** — a `Connect<Provider>` gRPC (message in
   `models.proto`, rpc in `connectors.proto`, handler in
   `api/connectors/service.rs`) so a tenant can store the credentials, and the
   ability to set `invoicing_entity.tax_provider_id` to that connector.
7. **Frontend** (`web-app/`): a tax-provider integration card in
   `settings/tabs/IntegrationsTab.tsx` + a config modal under
   `settings/integrations/`, and a control to select the provider for an
   invoicing entity. Reuse existing components before building new UI — ask
   before building custom tables/comparison UIs.

Rules the abstraction depends on (repeated because they're easy to violate):
- Unsupported/failed calc → return a `TaxEngineError`, **never `panic!`** and
  never silently return zero tax.
- No provider-specific type leaks past the engine — the outside world only sees
  `CalculationResult` / `CustomerTax` / `VatNumberExternalValidationResult`.
- Currency precision from the currency, never hardcoded decimals.
- Sandbox-default any live/sandbox toggle so a malformed config never routes a
  real tax calculation to the wrong environment.

---

## Phase 4 — Verify

- Unit-test the engine against recorded provider responses (fixtures from the
  research doc), asserting the `TaxBreakdownItem` totals and rounding.
- Test the category map: every built-in `tax_category` key resolves to a provider
  tax code (or the documented fallback).
- Exercise `build_tax_engine` with an entity whose `tax_provider_id` is set,
  proving the connector loads and the right engine is returned.
- `cargo build` and re-grep the existing providers (`grep -rin gocardless`,
  `grep -rin stancer`) — every compiler-flagged non-exhaustive match arm is a
  site you must handle.
- Run the billing-reviewer agent over the tax-calculation path before finishing.

---

## Guardrails

- **Research before code.** If asked to "just add <provider> fast", still produce
  the research doc first — it *is* the fast path; guessing endpoints and rounding
  costs more later.
- **Official sources win.** Cite them. Flag any reliance on unofficial clients.
- **Never assume 2-decimal currency.** Convert with the currency's precision.
- **Fail loud, not to zero tax.** A calc error must surface as an error, not a
  zero-rate invoice line.
- Sandbox-default any live/sandbox toggle.
