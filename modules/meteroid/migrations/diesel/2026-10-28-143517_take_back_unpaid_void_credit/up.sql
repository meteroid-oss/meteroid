-- Before the credit ledger, voiding an unpaid invoice credited its whole total to the customer
-- balance (and a credit-to-balance note could credit an unpaid invoice): the carry-over turned that
-- unpaid value into spendable CUSTOMER_BALANCE credit, reported as revenue on the upgrade day. Take
-- it back, per customer, as each note's REVERSAL dated with the carry-over so the revenue facts net
-- on that day. A customer needing a decision (credit moved since, a payment on the invoice, another
-- currency, the balance drawn after the note, wallet automation) is left as is, with a notice.
-- Every customer taken back or left is reported: run it in a rolled-back transaction to preview.
SET LOCAL lock_timeout = '5s';

DO
$$
    DECLARE
        candidate RECORD;
        cust      RECORD;
        g         RECORD;
        a         RECORD;
        note      RECORD;
        total     bigint;
        notes     int;
        customers int    := 0;
        moved     bigint := 0;
    BEGIN
        FOR candidate IN
            SELECT DISTINCT cn.tenant_id, cn.customer_id
            FROM credit_note cn
            JOIN invoice i ON i.id = cn.invoice_id AND i.tenant_id = cn.tenant_id
            JOIN customer_credit_grant gr ON gr.tenant_id = cn.tenant_id AND gr.source_kind = 'CUSTOMER_BALANCE'
                AND gr.source_id = cn.customer_id
            WHERE cn.credited_retired_balance
              AND cn.credit_type = 'CREDIT_TO_BALANCE'
              AND i.voided_at IS NOT NULL
            ORDER BY cn.tenant_id, cn.customer_id
            LOOP
                BEGIN
                    total := 0;
                    notes := 0;
                    -- The app's lock order: customer, then its grants.
                    SELECT tenant_id, id INTO cust FROM customer
                    WHERE tenant_id = candidate.tenant_id AND id = candidate.customer_id FOR NO KEY UPDATE;
                    SELECT * INTO g FROM customer_credit_grant
                    WHERE tenant_id = cust.tenant_id AND source_kind = 'CUSTOMER_BALANCE' AND source_id = cust.id
                    FOR NO KEY UPDATE;
                    SELECT * INTO a FROM customer_credit_account WHERE tenant_id = g.tenant_id AND id = g.account_id;

                    -- The customer's pre-ledger credit-to-balance notes of voided invoices, each with
                    -- its share of the invoice's credit beyond what the invoice drew from the balance
                    -- (the customer's own money).
                    IF to_regclass('pg_temp.free_note') IS NOT NULL THEN
                        DROP TABLE pg_temp.free_note;
                    END IF;
                    CREATE TEMP TABLE free_note ON COMMIT DROP AS
                    SELECT cn.id, cn.invoice_id, upper(trim(cn.currency)) AS currency,
                           least(cn.created_at, cn.finalized_at) AS issued_at, cn.finalized_at,
                           greatest(0, sum(cn.credited_amount_cents) OVER upto - greatest(i.applied_credits, 0))
                               - greatest(0, coalesce(sum(cn.credited_amount_cents) OVER before, 0)
                                             - greatest(i.applied_credits, 0)) AS unpaid
                    FROM credit_note cn
                    JOIN invoice i ON i.id = cn.invoice_id AND i.tenant_id = cn.tenant_id
                    WHERE cn.tenant_id = cust.tenant_id AND cn.customer_id = cust.id
                      AND cn.credited_retired_balance
                      AND cn.credit_type = 'CREDIT_TO_BALANCE'
                      AND i.voided_at IS NOT NULL
                    WINDOW upto AS (PARTITION BY i.id ORDER BY cn.finalized_at, cn.id
                                    ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW),
                           before AS (PARTITION BY i.id ORDER BY cn.finalized_at, cn.id
                                      ROWS BETWEEN UNBOUNDED PRECEDING AND 1 PRECEDING);

                    IF g.remaining <> g.amount OR g.cost_minor IS DISTINCT FROM g.amount
                        OR EXISTS (SELECT 1 FROM customer_credit_entry e WHERE e.grant_id = g.id AND e.kind <> 'FUND') THEN
                        RAISE EXCEPTION 'the carried-over credit moved since the upgrade';
                    END IF;
                    IF EXISTS (SELECT 1 FROM customer_credit_auto_top_up t WHERE t.account_id = a.id)
                        OR cardinality(a.low_balance_thresholds_minor) > 0
                        OR a.usage_at_zero = 'ALWAYS_PAUSE' THEN
                        RAISE EXCEPTION 'the wallet has auto top-up, low-balance levels or pauses usage at zero';
                    END IF;
                    IF EXISTS (SELECT 1
                               FROM free_note n
                               JOIN payment_transaction pt ON pt.tenant_id = cust.tenant_id AND pt.invoice_id = n.invoice_id
                               WHERE pt.status NOT IN ('FAILED', 'CANCELLED')) THEN
                        RAISE EXCEPTION 'a voided invoice has a payment';
                    END IF;
                    IF EXISTS (SELECT 1 FROM free_note n WHERE n.currency <> a.currency) THEN
                        RAISE EXCEPTION 'a note is not in the wallet currency %', a.currency;
                    END IF;
                    -- Drawn after such a note, the balance may have spent that credit and kept genuine credit.
                    IF EXISTS (SELECT 1
                               FROM customer_balance_tx t
                               WHERE t.customer_id = cust.id AND t.amount_cents < 0
                                 AND t.created_at >= (SELECT min(n.issued_at) AT TIME ZONE 'UTC' FROM free_note n)
                                                     - interval '1 day') THEN
                        RAISE EXCEPTION 'the balance was drawn after a voided invoice credited it';
                    END IF;

                    FOR note IN SELECT id, unpaid FROM free_note WHERE unpaid > 0 ORDER BY finalized_at, id
                        LOOP
                            INSERT INTO customer_credit_entry (id, tenant_id, customer_id, grant_id, kind, amount,
                                                               source_id, credit_note_id, occurred_at, cost_minor)
                            VALUES (uuidv7(), cust.tenant_id, cust.id, g.id, 'REVERSAL', -note.unpaid, note.id, note.id,
                                    g.effective_at, -note.unpaid);
                            total := total + note.unpaid;
                            notes := notes + 1;
                        END LOOP;

                    IF total = 0 THEN
                        RAISE EXCEPTION 'nothing unpaid to take back';
                    END IF;
                    IF total > g.remaining THEN
                        RAISE EXCEPTION 'would take back % of % remaining', total, g.remaining;
                    END IF;

                    UPDATE customer_credit_grant SET remaining = remaining - total WHERE id = g.id;
                    UPDATE customer_credit_account SET ledger_version = ledger_version + 1
                    WHERE tenant_id = cust.tenant_id AND customer_id = cust.id;
                    INSERT INTO credit_input_change (shard, tenant_id, customer_id, kind)
                    VALUES (credit_input_shard(cust.tenant_id), cust.tenant_id, cust.id, 'LEDGER')
                    ON CONFLICT DO NOTHING;
                    INSERT INTO credit_recompute_mark (tenant_id, customer_id) VALUES (cust.tenant_id, cust.id)
                    ON CONFLICT (tenant_id, customer_id) DO UPDATE SET marked_at = clock_timestamp();

                    customers := customers + 1;
                    moved := moved + total;
                    RAISE NOTICE 'customer % (tenant %): taken back % % (% notes), % left',
                        cust.id, cust.tenant_id, total, a.currency, notes, g.remaining - total;
                EXCEPTION
                    WHEN raise_exception THEN
                        RAISE NOTICE 'customer % (tenant %): left as is, %', candidate.customer_id,
                            candidate.tenant_id, SQLERRM;
                END;
            END LOOP;

        RAISE NOTICE 'unpaid void credit taken back from % customers (% minor units)', customers, moved;
    END
$$;
