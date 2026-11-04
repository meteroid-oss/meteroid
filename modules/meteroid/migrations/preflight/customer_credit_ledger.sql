-- Read-only preflight for the 2026-10-08-110347_customer_credit_ledger carry-over. Run it against
-- production right BEFORE upgrading. The first result lists every row to reconcile first: those that
-- would make the migration abort (negative balances, credit purchases, which this release does not
-- carry over), and invoices of non-sandbox tenants already overpaid, which would mint their whole
-- excess as credit at their next payment event. The second gives the per-currency totals to
-- reconcile the carry-over with. It writes nothing; an empty first result means the cutover is clear.

WITH collected AS (
    -- Through to_jsonb, so the query runs on every release's schema: amount_reversed today,
    -- amount_refunded before 2026-09-21 (that migration moves it), neither before 2026-08-20.
    SELECT pt.tenant_id, pt.invoice_id,
           sum(pt.amount - coalesce((to_jsonb(pt) ->> 'amount_reversed')::bigint,
                                    (to_jsonb(pt) ->> 'amount_refunded')::bigint, 0)) AS amount
    FROM payment_transaction pt
    WHERE pt.payment_type = 'PAYMENT'
      AND pt.status::text IN ('SETTLED', 'REFUNDED')
      AND pt.invoice_id IS NOT NULL
    GROUP BY pt.tenant_id, pt.invoice_id),
blockers AS (
    SELECT 'negative balance' AS issue, c.tenant_id, c.id AS customer_id, NULL::uuid AS invoice_id,
           format('balance %s', c.balance_value_cents) AS detail
    FROM customer c
    WHERE c.balance_value_cents < 0
    UNION ALL
    SELECT 'credit purchase', p.tenant_id, p.customer_id, p.invoice_id, format('amount %s', p.amount_cents)
    FROM customer_balance_pending_tx p
    UNION ALL
    SELECT 'overpaid invoice', i.tenant_id, i.customer_id, i.id,
           format('collected %s of %s', col.amount, i.total)
    FROM invoice i
    JOIN collected col ON col.tenant_id = i.tenant_id AND col.invoice_id = i.id
    JOIN tenant t ON t.id = i.tenant_id
    WHERE col.amount > i.total
      AND t.environment <> 'SANDBOX')
SELECT * FROM blockers ORDER BY issue, tenant_id, customer_id;

-- Per-currency carry-over totals: the balances that become CUSTOMER_BALANCE grants.
SELECT upper(currency) AS currency, count(*) AS customers, sum(balance_value_cents) AS total_minor
FROM customer
WHERE balance_value_cents > 0
GROUP BY 1
ORDER BY 1;
