-- Card-collection campaigns: who is asked to add a payment method, when, and what happens to
-- those still without one at their deadline. They replace the fixed per-subscription reminders.
CREATE TABLE migration_campaign
(
    id                   UUID PRIMARY KEY,
    tenant_id            UUID        NOT NULL REFERENCES tenant ON DELETE CASCADE,
    source_id            UUID        NOT NULL REFERENCES migration_source ON DELETE CASCADE,
    name                 TEXT        NOT NULL,
    -- Manual requests and the reminders of before campaigns: one per source.
    is_default           BOOLEAN     NOT NULL DEFAULT FALSE,
    status               TEXT        NOT NULL DEFAULT 'DRAFT'
        CHECK (status IN ('DRAFT', 'SCHEDULED', 'RUNNING', 'PAUSED', 'COMPLETED', 'CANCELLED')),
    audience_filter      JSONB       NOT NULL DEFAULT '{}',
    -- Customers matching later join while it runs.
    audience_live        BOOLEAN     NOT NULL DEFAULT TRUE,
    starts_at            TIMESTAMPTZ,
    timezone             TEXT        NOT NULL DEFAULT 'UTC',
    -- Emails go out between these hours (local), on these ISO weekdays (1 = Monday).
    window_start_hour    SMALLINT    NOT NULL DEFAULT 9 CHECK (window_start_hour BETWEEN 0 AND 23),
    window_end_hour      SMALLINT    NOT NULL DEFAULT 17 CHECK (window_end_hour BETWEEN 1 AND 24),
    window_days          SMALLINT[]  NOT NULL DEFAULT '{1,2,3,4,5}',
    -- Days before the member's deadline a reminder goes out on.
    reminder_offsets     INTEGER[]   NOT NULL DEFAULT '{14,7,2}',
    max_emails           INTEGER     NOT NULL DEFAULT 4 CHECK (max_emails BETWEEN 1 AND 6),
    min_gap_days         INTEGER     NOT NULL DEFAULT 3 CHECK (min_gap_days >= 2),
    -- A member's deadline: this many days before the day their card is needed by.
    deadline_offset_days INTEGER     NOT NULL DEFAULT 2 CHECK (deadline_offset_days >= 0),
    deadline_outcome     TEXT        NOT NULL DEFAULT 'HOLD'
        CHECK (deadline_outcome IN ('HOLD', 'CUTOVER_INVOICE', 'CUTOVER_FLAG')),
    max_per_hour         INTEGER     NOT NULL DEFAULT 100 CHECK (max_per_hour > 0),
    max_per_day          INTEGER     NOT NULL DEFAULT 1000 CHECK (max_per_day > 0),
    created_by           UUID,
    updated_by           UUID,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    started_at           TIMESTAMPTZ,
    paused_at            TIMESTAMPTZ,
    cancelled_at         TIMESTAMPTZ,
    completed_at         TIMESTAMPTZ,
    CHECK (window_end_hour > window_start_hour)
);
CREATE INDEX migration_campaign_source_idx ON migration_campaign (source_id, created_at DESC);
CREATE UNIQUE INDEX migration_campaign_default_idx ON migration_campaign (source_id) WHERE is_default;

CREATE TABLE migration_campaign_member
(
    id                    UUID PRIMARY KEY,
    tenant_id             UUID        NOT NULL REFERENCES tenant ON DELETE CASCADE,
    campaign_id           UUID        NOT NULL REFERENCES migration_campaign ON DELETE CASCADE,
    migration_customer_id UUID        NOT NULL REFERENCES migration_customer ON DELETE CASCADE,
    status                TEXT        NOT NULL DEFAULT 'QUEUED'
        CHECK (status IN ('QUEUED', 'EMAILING', 'CARD_ADDED', 'HELD', 'CUT_OVER', 'EXCLUDED',
                          'OPTED_OUT', 'UNREACHABLE', 'LEFT')),
    joined_via            TEXT        NOT NULL CHECK (joined_via IN ('FILTER', 'MANUAL', 'MOVED', 'MIGRATED')),
    deadline              DATE,
    emails_sent           INTEGER     NOT NULL DEFAULT 0,
    last_emailed_at       TIMESTAMPTZ,
    next_send_at          TIMESTAMPTZ,
    -- When a portal link sent to them was first and last seen used.
    first_opened_at       TIMESTAMPTZ,
    last_opened_at        TIMESTAMPTZ,
    card_added_at         TIMESTAMPTZ,
    outcome               TEXT CHECK (outcome IN ('HOLD', 'CUTOVER_INVOICE', 'CUTOVER_FLAG')),
    outcome_applied_at    TIMESTAMPTZ,
    excluded_by           UUID,
    excluded_reason       TEXT,
    -- Their stop link's token: it only stops these reminders, like an unsubscribe link.
    stop_token            TEXT UNIQUE,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (campaign_id, migration_customer_id)
);
-- At most one campaign emails a customer.
CREATE UNIQUE INDEX migration_campaign_member_active_idx
    ON migration_campaign_member (migration_customer_id) WHERE status IN ('QUEUED', 'EMAILING');
CREATE INDEX migration_campaign_member_campaign_idx ON migration_campaign_member (campaign_id, status);

CREATE TABLE migration_campaign_send
(
    id          UUID PRIMARY KEY,
    tenant_id   UUID        NOT NULL REFERENCES tenant ON DELETE CASCADE,
    member_id   UUID        NOT NULL REFERENCES migration_campaign_member ON DELETE CASCADE,
    step        INTEGER     NOT NULL,
    queued_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    sent_at     TIMESTAMPTZ,
    skip_reason TEXT
);
CREATE INDEX migration_campaign_send_member_idx ON migration_campaign_send (member_id, queued_at DESC);
CREATE INDEX migration_campaign_send_sent_idx ON migration_campaign_send (tenant_id, sent_at);

-- Deadline outcomes, read by the state evaluation: held at the source, or cut over without a card.
ALTER TABLE migration_subscription
    ADD COLUMN payment_method_held   BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN payment_method_waived BOOLEAN NOT NULL DEFAULT FALSE;

ALTER TABLE migration_event
    ADD COLUMN campaign_id UUID REFERENCES migration_campaign ON DELETE SET NULL;
CREATE INDEX migration_event_campaign_idx ON migration_event (campaign_id, created_at DESC) WHERE campaign_id IS NOT NULL;

-- Each source's Default campaign, with the cadence reminders had, and their running reminders.
INSERT INTO migration_campaign (id, tenant_id, source_id, name, is_default, status, audience_live,
                                window_start_hour, window_end_hour, window_days, deadline_offset_days,
                                started_at)
SELECT gen_random_uuid(), s.tenant_id, s.id, 'Requested payment methods', TRUE, 'RUNNING', FALSE,
       0, 24, '{1,2,3,4,5,6,7}', 0, now()
FROM migration_source s;

INSERT INTO migration_campaign_member (id, tenant_id, campaign_id, migration_customer_id, status,
                                       joined_via, emails_sent, last_emailed_at)
SELECT gen_random_uuid(), c.tenant_id, c.id, r.migration_customer_id,
       CASE WHEN r.emails > 0 THEN 'EMAILING' ELSE 'QUEUED' END, 'MIGRATED', r.emails, r.emailed_at
FROM (SELECT ms.source_id, ms.migration_customer_id,
             max(ms.payment_method_emails) AS emails,
             max(ms.payment_method_emailed_at) AS emailed_at
      FROM migration_subscription ms
      WHERE ms.payment_method_reminders AND ms.migration_customer_id IS NOT NULL
      GROUP BY ms.source_id, ms.migration_customer_id) r
JOIN migration_campaign c ON c.source_id = r.source_id AND c.is_default;

ALTER TABLE migration_subscription
    DROP COLUMN payment_method_reminders,
    DROP COLUMN payment_method_emails,
    DROP COLUMN payment_method_emailed_at;
