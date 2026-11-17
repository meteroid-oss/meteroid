-- The reconcile watermark. A period ending on or before `reconcile_from` was settled by the
-- per-operation code: billing from facts never evaluates it again, and an issued line starting
-- before it is never billed against. NULL: every period is billed from facts.
SET LOCAL lock_timeout = '5s';

ALTER TABLE subscription ADD COLUMN reconcile_from DATE;

-- The start of the latest period whose recurring invoice is issued with no draft before it: every
-- period ending by then was billed on an issued invoice. A draft still open is billed from facts.
-- A consolidated child is issued with its parent, and is no draft once merged (as in check 14).
UPDATE subscription s
SET reconcile_from = w.settled
FROM (SELECT i.subscription_id, max(i.billing_period_start) AS settled
      FROM invoice i
      LEFT JOIN invoice p ON p.id = i.consolidated_into_invoice_id
      WHERE i.invoice_type = 'RECURRING'
        AND i.subscription_id IS NOT NULL
        AND CASE WHEN i.consolidated_into_invoice_id IS NULL
                 THEN i.status IN ('FINALIZED', 'UNCOLLECTIBLE')
                 ELSE p.status IN ('FINALIZED', 'UNCOLLECTIBLE') END
        AND NOT EXISTS (SELECT 1 FROM invoice d
                        WHERE d.subscription_id = i.subscription_id
                          AND d.invoice_type = 'RECURRING'
                          AND d.status = 'DRAFT'
                          AND d.consolidated_into_invoice_id IS NULL
                          AND d.billing_period_start <= i.billing_period_start)
      GROUP BY i.subscription_id) w
WHERE w.subscription_id = s.id;
