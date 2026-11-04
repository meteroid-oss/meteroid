-- Sets the frozen scalar balance to the live invoice credit, recorded as a balance transaction.
-- Anything the shipped schema cannot represent (credit units, purchases, top-ups, credit packs,
-- promotional, scheduled, expiring, usage or foreign-currency credit, split credit-note effects)
-- aborts the rollback.
LOCK TABLE customer, customer_balance_tx, customer_balance_pending_tx, invoice, credit_note
    IN SHARE ROW EXCLUSIVE MODE;

DROP TRIGGER customer_balance_pending_tx_frozen ON customer_balance_pending_tx;
DROP TRIGGER customer_balance_tx_frozen ON customer_balance_tx;
DROP TRIGGER customer_balance_value_seeded ON customer;
DROP TRIGGER customer_balance_value_frozen ON customer;
DROP FUNCTION customer_balance_frozen();

DO
$$
    BEGIN
        -- A credit unit is only ever archived, never deleted: one existing means some wallet, grant,
        -- price, budget or pack may name it.
        IF EXISTS (SELECT 1 FROM credit_unit) THEN
            RAISE EXCEPTION 'credit units exist; the shipped schema cannot represent what they price, fund or cap';
        END IF;
        -- Plan rollover has no column in the shipped schema, whether in money or in a credit unit.
        IF EXISTS (SELECT 1 FROM customer_credit_grant WHERE rollover IS NOT NULL OR source_kind = 'ROLLOVER')
            OR EXISTS (SELECT 1 FROM customer_credit_entry WHERE kind = 'ROLLOVER') THEN
            RAISE EXCEPTION 'rollover grants exist; the shipped schema cannot represent plan rollover';
        END IF;
        IF EXISTS (SELECT 1 FROM customer_credit_purchase) THEN
            RAISE EXCEPTION 'credit purchases exist; reconcile before rollback';
        END IF;
        IF EXISTS (SELECT 1 FROM credit_usage_settlement WHERE invoice_id IS NULL AND returned_at IS NULL) THEN
            RAISE EXCEPTION 'usage settled from credit ahead of its invoice exists; finalize the invoices before rollback';
        END IF;
        IF EXISTS (SELECT 1 FROM customer_credit_operation)
            OR EXISTS (SELECT 1 FROM payment_transaction WHERE payment_type::text IN ('TOP_UP', 'AUTO_TOPUP')) THEN
            RAISE EXCEPTION 'credit top-ups exist; reconcile before rollback to preserve payment evidence';
        END IF;
        IF EXISTS (SELECT 1 FROM product WHERE fee_type::text = 'CREDIT') THEN
            RAISE EXCEPTION 'credit packs exist; remove them before rollback';
        END IF;
        IF EXISTS (SELECT 1 FROM credit_note
                   WHERE credit_type <> 'DEBT_CANCELLATION' AND debt_cancelled_amount_cents <> 0) THEN
            RAISE EXCEPTION 'credit notes record a debt cancellation the shipped schema cannot hold';
        END IF;
        IF EXISTS (SELECT 1 FROM invoice WHERE line_items @> '[{"kind": "Credit"}]')
            OR EXISTS (SELECT 1 FROM credit_note WHERE line_items @> '[{"kind": "Credit"}]') THEN
            RAISE EXCEPTION 'credit lines exist that the shipped schema cannot read; reconcile before rollback';
        END IF;
        IF EXISTS (SELECT 1
                   FROM customer_credit_grant g
                   JOIN customer_credit_account a ON a.id = g.account_id
                   JOIN customer c ON c.id = g.customer_id AND c.tenant_id = g.tenant_id
                   WHERE g.remaining > 0
                     AND (g.expires_at IS NULL OR g.expires_at > now())
                     AND (g.application <> 'INVOICE' OR g.subscription_id IS NOT NULL
                          OR a.currency <> upper(c.currency))) THEN
            RAISE EXCEPTION 'customer credit exists that the scalar balance cannot hold; reconcile before rollback';
        END IF;
        -- The scalar pays any product: credit restricted to some would widen to all of them.
        IF EXISTS (SELECT 1
                   FROM customer_credit_grant g
                   JOIN customer_credit_grant_product p ON p.grant_id = g.id
                   WHERE g.remaining > 0
                     AND (g.expires_at IS NULL OR g.expires_at > now())) THEN
            RAISE EXCEPTION 'product-restricted customer credit exists that the scalar balance cannot hold; reconcile before rollback';
        END IF;
        -- The scalar is plain paid balance: promotional, scheduled or expiring credit would change value.
        IF EXISTS (SELECT 1
                   FROM customer_credit_grant g
                   WHERE g.remaining > 0
                     AND (g.expires_at IS NULL OR g.expires_at > now())
                     AND (g.classification <> 'PAID' OR g.effective_at > now() OR g.expires_at IS NOT NULL)) THEN
            RAISE EXCEPTION 'promotional, scheduled or expiring customer credit exists that the scalar balance cannot hold; reconcile before rollback';
        END IF;
    END
$$;

DELETE FROM checkout_session WHERE checkout_type::text = 'TOP_UP';
ALTER TABLE checkout_session ALTER COLUMN plan_version_id SET NOT NULL;

CREATE TEMPORARY TABLE rollback_fold ON COMMIT DROP AS
SELECT g.tenant_id,
       g.customer_id,
       gen_random_uuid() AS tx_id,
       coalesce(sum(g.remaining) FILTER (WHERE g.expires_at IS NULL OR g.expires_at > now()), 0)::bigint AS amount
FROM customer_credit_grant g
GROUP BY g.tenant_id, g.customer_id;

-- The scalar was frozen at its carried value: the live ledger balance replaces it.
INSERT INTO customer_balance_tx (id, amount_cents, balance_cents_after, note, tenant_id, customer_id, currency)
SELECT f.tx_id, f.amount - c.balance_value_cents, f.amount, 'Customer credit ledger rolled back', f.tenant_id,
       f.customer_id, upper(c.currency)
FROM rollback_fold f
JOIN customer c ON c.tenant_id = f.tenant_id AND c.id = f.customer_id;

UPDATE customer c
SET balance_value_cents = f.amount
FROM rollback_fold f
WHERE c.tenant_id = f.tenant_id
  AND c.id = f.customer_id
  AND c.balance_value_cents <> f.amount;
COMMENT ON COLUMN customer.balance_value_cents IS NULL;

ALTER TABLE customer_balance_tx DROP COLUMN currency;
COMMENT ON TABLE customer_balance_tx IS NULL;

DROP INDEX credit_note_finalized_refund_idx;
DROP INDEX payment_transaction_reversed_idx;

-- No top-up payment exists (checked above), so the shipped constraint validates.
ALTER TABLE payment_transaction
    DROP CONSTRAINT payment_transaction_invoice_or_checkout,
    ADD CONSTRAINT payment_transaction_invoice_or_checkout
        CHECK (invoice_id IS NOT NULL OR checkout_session_id IS NOT NULL);

ALTER TABLE invoicing_entity
    DROP COLUMN portal_budgets_enabled,
    DROP COLUMN portal_auto_top_up_enabled,
    DROP COLUMN portal_credit_top_up_enabled;

DROP TRIGGER credit_note_debt_cancelled_amount ON credit_note;
DROP FUNCTION credit_note_debt_cancelled_amount();
ALTER TABLE credit_note
    DROP COLUMN credited_retired_balance,
    DROP COLUMN debt_cancelled_amount_cents;

DROP TABLE usage_monitor_state;
DROP TABLE customer_budget;
DROP TABLE usage_limit;

DROP TABLE credit_input_change_retention;
DROP TABLE credit_input_change;
DROP TYPE "CreditInputChangeKindEnum";
DROP TABLE credit_tenant_inputs;
DROP TABLE credit_customer_inputs;
DROP FUNCTION credit_input_shard(UUID);
DROP FUNCTION credit_input_token();

SELECT pgmq.unbind_topic('subscription.created', 'credit_recompute');
SELECT pgmq.unbind_topic('subscription.updated', 'credit_recompute');
SELECT pgmq.unbind_topic('subscription.cancelled', 'credit_recompute');
SELECT pgmq.unbind_topic('subscription.ended', 'credit_recompute');
SELECT pgmq.unbind_topic('invoice.finalized', 'credit_recompute');
SELECT pgmq.unbind_topic('invoice.voided', 'credit_recompute');
SELECT pgmq.unbind_topic('credit_note.finalized', 'credit_recompute');
SELECT pgmq.unbind_topic('credit_note.voided', 'credit_recompute');
SELECT pgmq.unbind_topic('wallet.funded', 'credit_recompute');
SELECT pgmq.unbind_topic('wallet.debited', 'credit_recompute');
SELECT pgmq.unbind_topic('credit_grant.expired', 'credit_recompute');
SELECT pgmq.drop_queue('credit_recompute');
DROP TABLE credit_recompute_mark;

DROP TABLE usage_alert_state;
DROP TABLE credit_usage_settlement;
DROP TABLE usage_fold;
DROP TABLE credit_balance_projection;
DROP TABLE customer_credit_unrecovered;
DROP TABLE customer_credit_purchase;
DROP TABLE api_idempotency_key;
DROP TABLE customer_credit_auto_top_up;
DROP TABLE customer_credit_operation;
DROP TABLE analytics_projection_quarantine;
DROP TABLE customer_credit_line_allocation;
DROP TABLE customer_credit_entry;
DROP TABLE customer_credit_grant_product;
DROP TABLE customer_credit_grant;
DROP TABLE customer_credit_account;
DROP TABLE credit_unit_price;
DROP TABLE credit_unit;

DROP TYPE "BudgetBehaviorEnum";
DROP TYPE "BudgetAuthorEnum";
DROP TYPE "UsageLimitBehaviorEnum";

-- PostgreSQL cannot drop enum values: 'TOP_UP' and 'AUTO_TOPUP' on "PaymentTypeEnum", 'CREDIT' on
-- "FeeTypeEnum" and 'TOP_UP' on "CheckoutTypeEnum" remain, unused.
