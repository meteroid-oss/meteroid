
-- Splits entity_activity.actor_id (base62-encoded UUID or free-form text) into:
--   actor_uuid UUID  — for USER / API_TOKEN / CUSTOMER actors
--   actor_alias TEXT — for QUOTE_RECIPIENT actors (e.g. an email address)
--
-- base62 encoding used by BaseId::as_base62:
--   1. uuid.as_u128().rotate_left(67)
--   2. base62::encode  (alphabet: 0-9 A-Z a-z)
-- Decode: strip prefix → base62-decode → rotate_right(67) → uuid

CREATE OR REPLACE FUNCTION _meteroid_base62_to_uuid(encoded TEXT) RETURNS UUID AS $$
DECLARE
    alphabet CONSTANT TEXT := '0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz';
    payload  TEXT;
    val      NUMERIC := 0;
    i        INT;
    c        CHAR;
    pos      INT;
    -- powers needed for rotate_right(67) on 128-bit: (val >> 67) | (val << 61) mod 2^128
    pow67    CONSTANT NUMERIC := 147573952589676412928;   -- 2^67
    pow61    CONSTANT NUMERIC := 2305843009213693952;     -- 2^61
    pow128   CONSTANT NUMERIC := 340282366920938463463374607431768211456; -- 2^128
    rotated  NUMERIC;
    hex_str  TEXT := '';
    rem      INT;
    hex_chars CONSTANT TEXT := '0123456789abcdef';
BEGIN
    -- Strip prefix: everything up to and including the last '_'
    payload := regexp_replace(encoded, '^.*_', '');

    -- base62 decode
    FOR i IN 1..length(payload) LOOP
        c   := substr(payload, i, 1);
        pos := position(c IN alphabet) - 1;
        IF pos < 0 THEN
            RAISE EXCEPTION '_meteroid_base62_to_uuid: invalid char % in %', c, encoded;
        END IF;
        val := val * 62 + pos;
    END LOOP;

    -- undo rotate_left(67): rotate_right(67) on 128-bit
    -- Use div() not floor(a/b): floor() rounds the intermediate division result,
    -- causing off-by-one errors for large NUMERIC values near 2^128.
    rotated := (div(val, pow67) + ((val % pow67) * pow61)) % pow128;

    -- numeric → 32-hex-char string
    FOR i IN 1..32 LOOP
        rem     := (rotated % 16)::INT;
        hex_str := substr(hex_chars, rem + 1, 1) || hex_str;
        rotated := div(rotated, 16);
    END LOOP;

    RETURN (
        substr(hex_str,  1, 8) || '-' ||
        substr(hex_str,  9, 4) || '-' ||
        substr(hex_str, 13, 4) || '-' ||
        substr(hex_str, 17, 4) || '-' ||
        substr(hex_str, 21, 12)
    )::UUID;
END;
$$ LANGUAGE plpgsql IMMUTABLE;

-- Add new columns
ALTER TABLE entity_activity
    ADD COLUMN actor_uuid  UUID,
    ADD COLUMN actor_alias TEXT;

-- Populate from actor_id:
--   UUID-keyed actors (USER, API_TOKEN, CUSTOMER) → decode base62 → actor_uuid
--   QUOTE_RECIPIENT → copy text verbatim → actor_alias
--   SYSTEM → both remain NULL
UPDATE entity_activity
SET actor_uuid = _meteroid_base62_to_uuid(actor_id)
WHERE actor_type IN ('USER', 'API_TOKEN', 'CUSTOMER')
  AND actor_id IS NOT NULL;

UPDATE entity_activity
SET actor_alias = actor_id
WHERE actor_type = 'QUOTE_RECIPIENT'
  AND actor_id IS NOT NULL;

-- Drop old text index and column
DROP INDEX IF EXISTS idx_entity_activity_actor;
ALTER TABLE entity_activity DROP COLUMN actor_id;

-- Shape constraint: keeps UUID/alias/null aligned with actor_type
ALTER TABLE entity_activity
    ADD CONSTRAINT entity_activity_actor_shape CHECK (
        CASE actor_type
            WHEN 'SYSTEM'          THEN actor_uuid IS NULL     AND actor_alias IS NULL
            WHEN 'QUOTE_RECIPIENT' THEN actor_uuid IS NULL     AND actor_alias IS NOT NULL
            ELSE                        actor_uuid IS NOT NULL AND actor_alias IS NULL
        END
    );

-- Rebuild actor index on actor_uuid (SYSTEM and QUOTE_RECIPIENT have no uuid, so partial)
CREATE INDEX idx_entity_activity_actor
    ON entity_activity (tenant_id, actor_type, actor_uuid, occurred_at DESC)
    WHERE actor_uuid IS NOT NULL;

DROP FUNCTION _meteroid_base62_to_uuid(TEXT);
