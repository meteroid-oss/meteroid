# Critical Billing Security Remediation Design

Date: 2026-08-19
Status: Approved for implementation planning
Scope: The three critical findings confirmed at commit `863bb284`

## Context

Meteroid currently has three release-blocking security and billing-integrity defects:

1. REST and gRPC API-key authentication cache successful validation by API-token ID alone. A different secret carrying the same ID can therefore reuse a warm cache entry.
2. Connector secrets and OAuth PKCE verifiers are encrypted with ChaCha20-Poly1305 using a nonce derived from the encryption key. Every record encrypted with that key reuses the same nonce.
3. Invoice payment creation holds an uncommitted database transaction open while calling Stripe. A successful Stripe request followed by a timeout rolls back the local attempt, and a retry uses a new idempotency key, permitting a duplicate charge.

The installation model permits a coordinated restart. Mixed old and new Meteroid binaries do not need to operate against the same database during this upgrade, but existing encrypted data must remain readable and be migrated safely.

## Goals

- Make cached API-key authorization depend on the complete presented credential.
- Use a unique random nonce for every newly encrypted secret.
- Migrate all legacy encrypted connector and OAuth-verifier values before the service accepts traffic.
- Persist each payment attempt before contacting a payment provider.
- Make payment delivery asynchronous, retryable, and idempotent across process crashes, network timeouts, and duplicate queue delivery.
- Preserve tenant boundaries and existing public response types.
- Provide regression tests and operator-facing rollout documentation with each implementation change.

## Non-goals

- Correcting the previously reported high-, medium-, or low-severity findings.
- Supporting a rolling deployment containing both legacy and upgraded binaries.
- Rotating third-party provider credentials automatically.
- Replacing Argon2 API-key hashing or the existing PGMQ outbox infrastructure.
- Broadly redesigning invoices, payment methods, or provider abstractions beyond what durable attempts require.

## 1. API-Key Authentication

### Design

REST and gRPC will call one reusable API-key verification component. Parsing remains responsible for extracting the token ID and secret component, but the authentication cache key will be a cryptographic digest of the complete canonical credential rather than `ApiTokenId`.

The digest prevents plaintext credentials from being retained as cache keys. The cached value contains only the resolved organization, tenant, environment, and token identity. A cache hit is valid only for the exact credential that previously passed Argon2 verification.

The existing bounded size and expiry may remain for this critical change. Immediate distributed revocation is a separate authorization-cache improvement and is outside this scope.

### Failure behavior

- Missing, malformed, undecodable, and wrong-secret credentials return unauthenticated responses.
- Store lookup failures do not populate the success cache.
- A valid credential followed by another credential with the same token ID and a different secret must fail on both REST and gRPC.
- Logs must not include the raw credential, secret component, or credential digest.

### Tests

- Unit tests cover canonical parsing and stable credential fingerprinting.
- gRPC integration tests warm the cache with a valid key and reject a forged secret carrying the same ID.
- REST integration tests exercise the same warm-cache regression.
- Existing valid-key and malformed-key behavior remains covered.

## 2. Versioned Secret Encryption

### Ciphertext envelope

New ciphertext uses the textual form:

```text
mtr1:<24 lowercase nonce hex characters>:<ciphertext-and-tag hex>
```

`mtr1` identifies the envelope version. Encryption generates a fresh 96-bit nonce from the operating system CSPRNG for every call. The existing 32-byte application encryption key remains the ChaCha20-Poly1305 key. The authentication tag remains part of the AEAD ciphertext returned by the library.

Decryption accepts `mtr1` envelopes and the existing unprefixed legacy hexadecimal format. Legacy decryption exists only to enable the coordinated migration and is not used for new writes.

### Startup migration

Before opening API or worker listeners, the upgraded application runs an idempotent application-level migration using the configured encryption key:

1. Begin one database transaction.
2. Lock and enumerate connector rows whose `sensitive` value is present and not prefixed with `mtr1:`.
3. Decrypt each value with the legacy nonce algorithm and re-encrypt it as `mtr1`.
4. Lock and enumerate OAuth verifier rows whose `pkce_verifier` is not prefixed with `mtr1:` and rewrite them in the same way.
5. Commit only after every selected value was successfully authenticated, decoded, and rewritten.

If any value fails, the transaction rolls back and service startup fails closed. The migration logs only row counts and outcome, never ciphertexts or plaintexts. Re-running startup after a successful migration is a no-op.

### Operational security

Re-encryption prevents future nonce reuse but cannot erase information already exposed through an earlier database snapshot. Operators must rotate Stripe, HubSpot, Pennylane, and other persisted provider credentials after the upgraded service is healthy. Existing short-lived OAuth verifiers may be allowed to expire, but the stored rows are still migrated for deterministic behavior.

### Tests

- Repeated encryption of identical plaintext produces different envelopes.
- Every generated nonce has the required length and envelopes round-trip.
- Legacy known-answer ciphertext still decrypts.
- Malformed versions, nonce lengths, hex, and authentication tags fail closed.
- A database integration test migrates legacy connector and OAuth rows atomically.
- A migration failure leaves all selected rows unchanged.
- A second migration run reports zero legacy rows.

## 3. Durable Asynchronous Payment Attempts

### Submission transaction

Invoice payment submission becomes a short database transaction:

1. Lock the invoice and validate its status, amount due, payment method, and existing active transactions.
2. Create one `Pending` payment transaction with a new immutable `PaymentTransactionId`.
3. Enqueue a `PaymentAttemptRequested` outbox event carrying that transaction ID.
4. Commit both records atomically.
5. Return the existing `PaymentTransaction` response in `Pending` state.

No payment-provider network call occurs inside this transaction. The invoice lock and active-pending check continue to prevent concurrent submissions from creating multiple attempts.

### Worker delivery

An outbox consumer loads the durable attempt and all provider inputs, then calls the provider without holding a database transaction. The provider interface receives an explicit idempotency key derived solely from the immutable Meteroid transaction ID. Stripe receives a stable, namespaced value such as `meteroid-payment:<transaction-id>` on every delivery of that attempt.

The worker classifies provider outcomes:

- **Confirmed response:** transactionally bind the external provider intent ID and apply the returned status.
- **Definite decline or permanent request rejection:** transactionally mark the attempt `Failed` with a sanitized error classification.
- **Timeout, connection loss, or ambiguous transport failure:** keep the attempt `Pending`, return a retryable worker error, and redeliver with the identical idempotency key.

PGMQ remains responsible for retry scheduling and dead-letter handling. Duplicate deliveries are expected and safe.

### Webhook reconciliation

Stripe metadata continues to carry the Meteroid tenant and transaction IDs. Before applying a provider event, reconciliation verifies:

- tenant and transaction identity;
- external provider intent identity once known;
- expected amount and currency;
- that the requested state transition is legal.

If a webhook settles the transaction before the worker receives its HTTP response, the worker consolidation step treats the matching settled state as an idempotent success. Repeated and out-of-order matching events are no-ops. Conflicting provider IDs, amounts, currencies, or terminal states fail closed and are retained for investigation.

### Tests

- Submission commits a pending transaction and its outbox event without invoking the provider.
- Concurrent submissions create at most one active attempt for an invoice.
- Duplicate worker deliveries use the same provider idempotency key.
- A simulated provider success with a lost response, followed by retry, creates one provider intent and settles one local transaction.
- A definite decline marks the durable attempt failed.
- Ambiguous failures remain pending and retryable.
- Webhook-before-response and response-before-webhook races converge on the same state.
- Mismatched provider ID, amount, or currency is rejected.

## Rollout and Rollback

The deployment procedure is intentionally restart-based:

1. Back up the database and verify the backup can be read.
2. Stop all legacy Meteroid API, scheduler, and worker instances.
3. Deploy the upgraded binary and schema migrations.
4. Start one upgraded instance so the application-level secret migration runs before listeners open.
5. Verify that no legacy encrypted values remain and that API/worker health checks pass.
6. Start the remaining upgraded instances.
7. Verify API-key rejection tests, queue health, pending-payment processing, and Stripe webhook reconciliation.
8. Rotate persisted provider credentials and verify each connector.

The legacy binary cannot decrypt `mtr1` values. Rollback therefore requires either continuing with a compatible upgraded binary or restoring the verified pre-migration database backup before starting the legacy version.

## Commit and Verification Strategy

Implementation is divided into three security commits after this design and its implementation plan:

1. API-key credential-bound caching, tests, and authentication documentation.
2. Versioned random-nonce encryption, startup migration, tests, and upgrade documentation.
3. Durable asynchronous payment attempts, worker/reconciliation tests, and payment-operation documentation.

Before each implementation commit, the focused regression tests must demonstrate the old behavior failing and the new behavior passing. After each atomic commit, run the relevant focused tests, formatting, a workspace build, and `cargo audit`. Pre-existing host or advisory failures are recorded explicitly and never reported as passing.
