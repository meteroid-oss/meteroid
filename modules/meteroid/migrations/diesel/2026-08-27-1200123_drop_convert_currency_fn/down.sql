create or replace function convert_currency(amount numeric, source_rate_from_usd numeric, target_rate_from_usd numeric) returns numeric
  language plpgsql
as
$$
DECLARE
  conversion_rate NUMERIC;
BEGIN
  conversion_rate := target_rate_from_usd / source_rate_from_usd;
  RETURN amount * conversion_rate;
END;
$$;
