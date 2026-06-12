
ALTER TABLE entity_activity DROP CONSTRAINT IF EXISTS entity_activity_actor_shape;

DROP INDEX IF EXISTS idx_entity_activity_actor;

ALTER TABLE entity_activity ADD COLUMN actor_id TEXT;

-- Re-encode actor_uuid back to a meteroid base62 ID.
-- Mirrors BaseId::as_base62: uuid → u128 → rotate_left(67) → base62 → prepend prefix.
CREATE OR REPLACE FUNCTION _meteroid_uuid_to_actor_id(u UUID, actor_type TEXT) RETURNS TEXT AS $$
DECLARE
    alphabet  CONSTANT TEXT    := '0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz';
    -- rotate_left(val, 67): (val % 2^61) * 2^67 + div(val, 2^61)
    -- sum is always ≤ 2^128-1 so no mod needed
    pow67     CONSTANT NUMERIC := 147573952589676412928;   -- 2^67
    pow61     CONSTANT NUMERIC := 2305843009213693952;     -- 2^61
    hex_str   TEXT;
    val       NUMERIC;
    rotated   NUMERIC;
    rem       INT;
    encoded   TEXT := '';
    prefix    TEXT;
BEGIN
    -- uuid → 32-char hex → NUMERIC (u128)
    hex_str := replace(u::TEXT, '-', '');
    val := 0;
    FOR i IN 1..32 LOOP
        val := val * 16 + (
            position(substr(hex_str, i, 1) IN '0123456789abcdef') - 1
        );
    END LOOP;

    -- rotate_left(val, 67) on 128-bit
    rotated := (val % pow61) * pow67 + div(val, pow61);

    -- base62 encode
    IF rotated = 0 THEN
        encoded := '0';
    ELSE
        WHILE rotated > 0 LOOP
            rem     := (rotated % 62)::INT;
            encoded := substr(alphabet, rem + 1, 1) || encoded;
            rotated := div(rotated, 62);
        END LOOP;
    END IF;

    -- prefix by actor_type
    prefix := CASE actor_type
        WHEN 'USER'      THEN 'usr_'
        WHEN 'API_TOKEN' THEN 'tkn_'
        WHEN 'CUSTOMER'  THEN 'cus_'
        ELSE ''
    END;

    RETURN prefix || encoded;
END;
$$ LANGUAGE plpgsql IMMUTABLE;

UPDATE entity_activity
SET actor_id = _meteroid_uuid_to_actor_id(actor_uuid, actor_type::TEXT)
WHERE actor_type IN ('USER', 'API_TOKEN', 'CUSTOMER')
  AND actor_uuid IS NOT NULL;

UPDATE entity_activity
SET actor_id = actor_alias
WHERE actor_type = 'QUOTE_RECIPIENT'
  AND actor_alias IS NOT NULL;

DROP FUNCTION _meteroid_uuid_to_actor_id(UUID, TEXT);

ALTER TABLE entity_activity
    DROP COLUMN actor_uuid,
    DROP COLUMN actor_alias;

CREATE INDEX idx_entity_activity_actor
    ON entity_activity (tenant_id, actor_type, actor_id, occurred_at DESC)
    WHERE actor_id IS NOT NULL;
