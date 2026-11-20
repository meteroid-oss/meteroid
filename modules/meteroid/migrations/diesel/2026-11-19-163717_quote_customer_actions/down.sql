ALTER TABLE quote_comment DROP COLUMN anchor;

ALTER TABLE quote
    DROP COLUMN last_sent_at,
    DROP COLUMN allow_decline,
    DROP COLUMN allow_change_requests,
    DROP COLUMN allow_questions;
