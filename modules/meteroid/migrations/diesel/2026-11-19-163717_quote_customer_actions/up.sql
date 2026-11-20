-- What a customer recipient may do in the portal besides signing.
ALTER TABLE quote
    ADD COLUMN allow_questions       BOOLEAN NOT NULL DEFAULT true,
    ADD COLUMN allow_change_requests BOOLEAN NOT NULL DEFAULT true,
    ADD COLUMN allow_decline         BOOLEAN NOT NULL DEFAULT true,
    ADD COLUMN last_sent_at          TIMESTAMPTZ;

-- The quote line a comment is about, by its name when written.
ALTER TABLE quote_comment ADD COLUMN anchor TEXT;

-- Sends before the column existed: each one is in the activity feed.
UPDATE quote q
SET last_sent_at = sent.at
FROM (
    SELECT a.tenant_id, a.entity_id, max(a.occurred_at) AS at
    FROM entity_activity a
    JOIN quote q2 ON q2.tenant_id = a.tenant_id AND q2.id = a.entity_id
    WHERE a.entity_type = 'quote' AND a.activity_type = 'quote.sent'
    GROUP BY a.tenant_id, a.entity_id
) sent
WHERE q.tenant_id = sent.tenant_id AND q.id = sent.entity_id;
