ALTER TYPE "StoredObjectKindEnum" ADD VALUE IF NOT EXISTS 'QUOTE_CERTIFICATE_PDF';

CREATE TYPE "QuoteSignatureLevelEnum" AS ENUM ('SIMPLE', 'VERIFIED');
CREATE TYPE "QuoteSignerRoleEnum" AS ENUM ('CUSTOMER', 'COUNTERSIGNER');
CREATE TYPE "QuoteApprovalStatusEnum" AS ENUM ('NOT_REQUIRED', 'PENDING', 'APPROVED', 'REJECTED');

-- Reusable starting points for new quotes: the editor state without the deal-specific fields.
CREATE TABLE quote_template (
    id                 UUID PRIMARY KEY,
    tenant_id          UUID NOT NULL REFERENCES tenant(id) ON DELETE CASCADE,
    name               TEXT NOT NULL,
    description        TEXT,
    payload            JSONB NOT NULL,
    expires_after_days INTEGER CHECK (expires_after_days > 0),
    created_by         UUID,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    archived_at        TIMESTAMPTZ
);
CREATE INDEX quote_template_tenant_idx ON quote_template (tenant_id) WHERE archived_at IS NULL;

ALTER TABLE quote
    ADD COLUMN revision             INTEGER NOT NULL DEFAULT 1,
    ADD COLUMN document_hash        TEXT,
    ADD COLUMN published_at         TIMESTAMPTZ,
    ADD COLUMN first_viewed_at      TIMESTAMPTZ,
    ADD COLUMN owner_id             UUID,
    ADD COLUMN template_id          UUID REFERENCES quote_template(id) ON DELETE SET NULL,
    ADD COLUMN decline_reason       TEXT,
    ADD COLUMN display_options      JSONB NOT NULL DEFAULT '{}',
    ADD COLUMN sections             JSONB NOT NULL DEFAULT '[]',
    ADD COLUMN signature_level      "QuoteSignatureLevelEnum" NOT NULL DEFAULT 'SIMPLE',
    ADD COLUMN consent_text         TEXT,
    ADD COLUMN countersigners       JSONB NOT NULL DEFAULT '[]',
    ADD COLUMN approval_status      "QuoteApprovalStatusEnum" NOT NULL DEFAULT 'NOT_REQUIRED',
    ADD COLUMN hubspot_deal_id      TEXT,
    ADD COLUMN max_discount_percent NUMERIC;

UPDATE quote SET published_at = COALESCE(updated_at, created_at) WHERE status <> 'DRAFT';

CREATE INDEX idx_quote_template_id ON quote (template_id) WHERE template_id IS NOT NULL;
CREATE INDEX idx_quote_owner_id ON quote (tenant_id, owner_id);

ALTER TABLE quote_component
    ADD COLUMN description      TEXT,
    ADD COLUMN sort_order       INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN discount_percent NUMERIC CHECK (discount_percent > 0 AND discount_percent <= 100),
    ADD COLUMN list_fee         JSONB;

ALTER TABLE quote_add_on
    ADD COLUMN description      TEXT,
    ADD COLUMN sort_order       INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN discount_percent NUMERIC CHECK (discount_percent > 0 AND discount_percent <= 100),
    ADD COLUMN list_fee         JSONB;

ALTER TABLE quote_signature
    ADD COLUMN signer_role     "QuoteSignerRoleEnum" NOT NULL DEFAULT 'CUSTOMER',
    ADD COLUMN revision        INTEGER,
    ADD COLUMN document_hash   TEXT,
    ADD COLUMN consent_text    TEXT,
    ADD COLUMN otp_verified_at TIMESTAMPTZ;

-- Frozen content of each published revision. The current revision is the quote itself.
CREATE TABLE quote_revision (
    id            UUID PRIMARY KEY,
    tenant_id     UUID NOT NULL REFERENCES tenant(id) ON DELETE CASCADE,
    quote_id      UUID NOT NULL REFERENCES quote(id) ON DELETE CASCADE,
    revision      INTEGER NOT NULL,
    snapshot      JSONB NOT NULL,
    document_hash TEXT NOT NULL,
    created_by    TEXT,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    superseded_at TIMESTAMPTZ,
    UNIQUE (quote_id, revision)
);

-- Unsaved editor state: one shared draft per existing quote, any number for new quotes.
CREATE TABLE quote_draft (
    id              UUID PRIMARY KEY,
    tenant_id       UUID NOT NULL REFERENCES tenant(id) ON DELETE CASCADE,
    quote_id        UUID REFERENCES quote(id) ON DELETE CASCADE,
    created_by      UUID NOT NULL REFERENCES "user"(id) ON DELETE CASCADE,
    updated_by      UUID REFERENCES "user"(id) ON DELETE SET NULL,
    label           TEXT,
    payload         JSONB NOT NULL,
    payload_version INTEGER NOT NULL DEFAULT 1,
    version         INTEGER NOT NULL DEFAULT 1,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX quote_draft_quote_idx ON quote_draft (quote_id) WHERE quote_id IS NOT NULL;
CREATE INDEX quote_draft_tenant_idx ON quote_draft (tenant_id, updated_at DESC);

-- Append-only, hash-chained record of what happened around a quote's signature.
CREATE TABLE quote_evidence_event (
    id            UUID PRIMARY KEY,
    tenant_id     UUID NOT NULL REFERENCES tenant(id) ON DELETE CASCADE,
    quote_id      UUID NOT NULL REFERENCES quote(id) ON DELETE CASCADE,
    seq           INTEGER NOT NULL,
    kind          TEXT NOT NULL,
    actor_email   TEXT,
    ip_address    TEXT,
    user_agent    TEXT,
    revision      INTEGER NOT NULL,
    document_hash TEXT,
    details       JSONB NOT NULL DEFAULT '{}',
    occurred_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    prev_hash     TEXT,
    hash          TEXT NOT NULL,
    UNIQUE (quote_id, seq)
);

CREATE TABLE quote_signature_otp (
    id          UUID PRIMARY KEY,
    tenant_id   UUID NOT NULL REFERENCES tenant(id) ON DELETE CASCADE,
    quote_id    UUID NOT NULL REFERENCES quote(id) ON DELETE CASCADE,
    email       TEXT NOT NULL,
    code_hash   TEXT NOT NULL,
    attempts    INTEGER NOT NULL DEFAULT 0,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at  TIMESTAMPTZ NOT NULL,
    verified_at TIMESTAMPTZ,
    consumed_at TIMESTAMPTZ
);
CREATE INDEX quote_signature_otp_lookup_idx ON quote_signature_otp (quote_id, email, created_at DESC);

-- The completion certificate of an accepted quote: what was signed, by whom, and the seal over it.
CREATE TABLE quote_signature_certificate (
    quote_id            UUID PRIMARY KEY REFERENCES quote(id) ON DELETE CASCADE,
    tenant_id           UUID NOT NULL REFERENCES tenant(id) ON DELETE CASCADE,
    revision            INTEGER NOT NULL,
    document_hash       TEXT NOT NULL,
    evidence_hash       TEXT NOT NULL,
    pdf_document_id     UUID,
    pdf_hash            TEXT,
    seal_key_id         TEXT,
    seal_signature      TEXT,
    timestamp_authority TEXT,
    timestamp_token     TEXT,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE quote_approval_rule (
    id                   UUID PRIMARY KEY,
    tenant_id            UUID NOT NULL REFERENCES tenant(id) ON DELETE CASCADE,
    name                 TEXT NOT NULL,
    min_discount_percent NUMERIC CHECK (min_discount_percent >= 0 AND min_discount_percent <= 100),
    max_net_terms_days   INTEGER CHECK (max_net_terms_days >= 0),
    max_trial_days       INTEGER CHECK (max_trial_days >= 0),
    always               BOOLEAN NOT NULL DEFAULT false,
    approver_user_ids    UUID[] NOT NULL DEFAULT '{}',
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    archived_at          TIMESTAMPTZ
);
CREATE INDEX quote_approval_rule_tenant_idx ON quote_approval_rule (tenant_id) WHERE archived_at IS NULL;

CREATE TABLE quote_approval (
    id             UUID PRIMARY KEY,
    tenant_id      UUID NOT NULL REFERENCES tenant(id) ON DELETE CASCADE,
    quote_id       UUID NOT NULL REFERENCES quote(id) ON DELETE CASCADE,
    revision       INTEGER NOT NULL,
    document_hash  TEXT NOT NULL,
    rule_ids       UUID[] NOT NULL,
    status         "QuoteApprovalStatusEnum" NOT NULL,
    requested_by   UUID,
    requested_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    request_note   TEXT,
    decided_by     UUID,
    decided_at     TIMESTAMPTZ,
    decision_note  TEXT
);
CREATE INDEX quote_approval_quote_idx ON quote_approval (quote_id, requested_at DESC);

-- The discussion on a quote: team notes, messages shared with the customer, change requests.
CREATE TABLE quote_comment (
    id                UUID PRIMARY KEY,
    tenant_id         UUID NOT NULL REFERENCES tenant(id) ON DELETE CASCADE,
    quote_id          UUID NOT NULL REFERENCES quote(id) ON DELETE CASCADE,
    revision          INTEGER NOT NULL,
    author_user_id    UUID,
    author_email      TEXT,
    author_name       TEXT NOT NULL,
    body              TEXT NOT NULL CHECK (length(body) BETWEEN 1 AND 10000),
    shared            BOOLEAN NOT NULL,
    is_change_request BOOLEAN NOT NULL DEFAULT false,
    resolved_at       TIMESTAMPTZ,
    resolved_by       UUID,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (shared OR NOT is_change_request)
);
CREATE INDEX quote_comment_quote_idx ON quote_comment (quote_id, created_at);
CREATE INDEX quote_comment_open_requests_idx ON quote_comment (quote_id)
    WHERE is_change_request AND resolved_at IS NULL;

ALTER TABLE quote ADD COLUMN changes_requested_at TIMESTAMPTZ;
