--! insert_rates
INSERT INTO historical_rates_from_usd (id, date, rates)
VALUES (:id, :date, :rates)
ON CONFLICT (date) DO UPDATE SET rates = EXCLUDED.rates
RETURNING id, date, rates;

--! get_rates
SELECT rates
FROM historical_rates_from_usd
WHERE date = :date;
