-- The SQL convert_currency() multiplied a minor-unit amount by a major-unit FX rate with no
-- minor-unit exponent correction, so cross-exponent conversions were off by a power of ten.
-- Its only caller (the pending-invoices stat) now converts in Rust via the single FX path, so
-- the function is unused and removed to avoid a second, drift-prone conversion path.
DROP FUNCTION IF EXISTS convert_currency(numeric, numeric, numeric);
