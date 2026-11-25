-- A v2 write's kept response is replayed with the content type it was answered with (ADR-0013 §3):
-- a problem stays `application/problem+json`. NULL on the responses kept before, read as JSON.
SET LOCAL lock_timeout = '5s';

ALTER TABLE api_idempotency_key ADD COLUMN response_content_type TEXT;
