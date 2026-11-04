//! Money the previous release moved back without the credit ledger: reversed or refunded payments
//! and finalized cash-refund credit notes of credit purchases, and payments kept whole as credit,
//! whose ledger has not followed.
use crate::errors::IntoDbResult;
use crate::{DbResult, PgConn};
use common_domain::ids::{
    BaseId, CreditNoteId, InvoiceId, PaymentTransactionId, TenantId, WalletId,
};
use diesel::sql_types::{BigInt, Bool, Uuid as SqlUuid};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

#[derive(Debug, Clone, Copy, diesel::QueryableByName)]
pub struct ReversedPurchaseInvoice {
    #[diesel(sql_type = SqlUuid)]
    pub tenant_id: TenantId,
    #[diesel(sql_type = SqlUuid)]
    pub invoice_id: InvoiceId,
    #[diesel(sql_type = Bool)]
    pub out_of_sync: bool,
}

#[derive(Debug, Clone, Copy, diesel::QueryableByName)]
pub struct UnreversedRefundNote {
    #[diesel(sql_type = SqlUuid)]
    pub tenant_id: TenantId,
    #[diesel(sql_type = SqlUuid)]
    pub credit_note_id: CreditNoteId,
}

impl ReversedPurchaseInvoice {
    /// The next `limit` invoices with a reversal, a purchase refunded without a note or a reported
    /// loss after the cursor, flagged when out of line with the target
    /// `reconcile_purchased_credit_tx` derives. Each is returned so the last is the next cursor: a
    /// page aggregates only its own invoices.
    pub async fn scan(
        conn: &mut PgConn,
        after: (TenantId, InvoiceId),
        limit: i64,
    ) -> DbResult<Vec<Self>> {
        diesel::sql_query(
            "WITH keys AS (
                 SELECT tenant_id, invoice_id FROM (
                     (SELECT DISTINCT tenant_id, invoice_id
                      FROM payment_transaction
                      WHERE amount_reversed > 0
                        AND invoice_id IS NOT NULL
                        AND status IN ('SETTLED', 'REFUNDED')
                        AND payment_type IN ('PAYMENT', 'TOP_UP', 'AUTO_TOPUP')
                        AND (tenant_id, invoice_id) > ($1, $2)
                      ORDER BY tenant_id, invoice_id
                      LIMIT $3)
                     UNION
                     (SELECT DISTINCT g.tenant_id, g.invoice_id
                      FROM customer_credit_grant g
                      JOIN payment_transaction r ON r.tenant_id = g.tenant_id AND r.invoice_id = g.invoice_id
                      WHERE g.source_kind = 'CREDIT_PURCHASE'
                        AND g.invoice_id IS NOT NULL
                        AND r.payment_type = 'REFUND'
                        AND r.credit_note_id IS NULL
                        AND r.amount > r.refund_excess
                        AND r.status IN ('READY', 'PENDING', 'SETTLED')
                        AND (g.tenant_id, g.invoice_id) > ($1, $2)
                      ORDER BY g.tenant_id, g.invoice_id
                      LIMIT $3)
                     UNION
                     (SELECT DISTINCT tenant_id, invoice_id
                      FROM customer_credit_entry
                      WHERE kind IN ('REVERSAL', 'REINSTATEMENT')
                        AND invoice_id IS NOT NULL
                        AND (tenant_id, invoice_id) > ($1, $2)
                      ORDER BY tenant_id, invoice_id
                      LIMIT $3)
                     UNION
                     (SELECT DISTINCT tenant_id, invoice_id
                      FROM customer_credit_unrecovered
                      WHERE source_id = invoice_id
                        AND reported <> 0
                        AND (tenant_id, invoice_id) > ($1, $2)
                      ORDER BY tenant_id, invoice_id
                      LIMIT $3)) candidate
                 ORDER BY tenant_id, invoice_id
                 LIMIT $3),
             reversed AS (
                 SELECT p.tenant_id, p.invoice_id, sum(p.amount_reversed)::bigint AS reversed,
                        sum(p.amount)::bigint AS gross,
                        max(p.store_committed_at) FILTER (WHERE p.amount_reversed > 0) AS touched_at
                 FROM payment_transaction p
                 JOIN keys k ON k.tenant_id = p.tenant_id AND k.invoice_id = p.invoice_id
                 WHERE p.invoice_id IS NOT NULL
                   AND p.status IN ('SETTLED', 'REFUNDED')
                   AND p.payment_type IN ('PAYMENT', 'TOP_UP', 'AUTO_TOPUP')
                 GROUP BY p.tenant_id, p.invoice_id),
             refunded AS (
                 SELECT r.tenant_id, r.invoice_id,
                        coalesce(sum(r.amount) FILTER (WHERE r.credit_note_id IS NOT NULL AND r.status = 'SETTLED'), 0)::bigint AS against_notes,
                        coalesce(sum(r.refund_excess) FILTER (WHERE r.credit_note_id IS NULL AND r.status IN ('READY', 'PENDING', 'SETTLED')), 0)::bigint AS excess_refunded,
                        coalesce(sum(r.amount - r.refund_excess) FILTER (WHERE r.credit_note_id IS NULL AND r.status IN ('READY', 'PENDING', 'SETTLED')), 0)::bigint AS refunded,
                        max(r.store_committed_at) FILTER (WHERE r.credit_note_id IS NULL AND r.amount > r.refund_excess) AS touched_at
                 FROM payment_transaction r
                 JOIN keys k ON k.tenant_id = r.tenant_id AND k.invoice_id = r.invoice_id
                 WHERE r.payment_type = 'REFUND'
                 GROUP BY r.tenant_id, r.invoice_id),
             moved AS (
                 SELECT e.tenant_id, e.invoice_id, e.grant_id,
                        -coalesce(sum(e.amount) FILTER (WHERE e.kind = 'REINSTATEMENT' OR e.credit_note_id IS NULL), 0)::bigint AS clawed,
                        -coalesce(sum(e.amount) FILTER (WHERE e.kind = 'REVERSAL' AND e.credit_note_id IS NOT NULL), 0)::bigint AS by_notes
                 FROM customer_credit_entry e
                 JOIN keys k ON k.tenant_id = e.tenant_id AND k.invoice_id = e.invoice_id
                 WHERE e.kind IN ('REVERSAL', 'REINSTATEMENT')
                   AND e.invoice_id IS NOT NULL
                 GROUP BY e.tenant_id, e.invoice_id, e.grant_id),
             reported AS (
                 SELECT u.tenant_id, u.invoice_id, u.account_id, u.reported
                 FROM customer_credit_unrecovered u
                 JOIN keys k ON k.tenant_id = u.tenant_id AND k.invoice_id = u.invoice_id
                 WHERE u.source_id = u.invoice_id),
             purchase AS (
                 SELECT g.tenant_id, g.invoice_id, g.id AS grant_id, g.account_id, g.amount,
                        g.remaining, g.created_at, g.classification
                 FROM customer_credit_grant g
                 JOIN keys k ON k.tenant_id = g.tenant_id AND k.invoice_id = g.invoice_id
                 WHERE g.source_kind = 'CREDIT_PURCHASE'),
             invoices AS (
                 SELECT tenant_id, invoice_id, min(created_at) AS since
                 FROM purchase
                 GROUP BY tenant_id, invoice_id),
             reduced AS (
                 SELECT cn.tenant_id, cn.invoice_id,
                        sum(CASE WHEN cn.credit_type = 'CREDIT_TO_BALANCE'
                                 THEN cn.debt_cancelled_amount_cents
                                 ELSE abs(cn.total) END)::bigint AS reduced,
                        coalesce(sum(cn.refunded_amount_cents) FILTER (WHERE cn.credit_type = 'REFUND'), 0)::bigint AS refundable
                 FROM credit_note cn
                 JOIN invoices iv ON iv.tenant_id = cn.tenant_id AND iv.invoice_id = cn.invoice_id
                 WHERE cn.status = 'FINALIZED'
                 GROUP BY cn.tenant_id, cn.invoice_id),
             money AS (
                 SELECT iv.tenant_id, iv.invoice_id, iv.since, greatest(r.touched_at, rf.touched_at) AS touched_at,
                        i.total - i.applied_credits - coalesce(f.reduced, 0) AS owed,
                        coalesce(r.gross, 0) - coalesce(rf.against_notes, 0) AS gross,
                        coalesce(r.reversed, 0) AS reversed,
                        coalesce(rf.excess_refunded, 0) AS excess_refunded,
                        coalesce(rf.refunded, 0) AS refunded,
                        greatest(coalesce(f.refundable, 0) - coalesce(rf.against_notes, 0), 0) AS owed_back
                 FROM invoices iv
                 JOIN invoice i ON i.tenant_id = iv.tenant_id AND i.id = iv.invoice_id
                 LEFT JOIN reversed r ON r.tenant_id = iv.tenant_id AND r.invoice_id = iv.invoice_id
                 LEFT JOIN refunded rf ON rf.tenant_id = iv.tenant_id AND rf.invoice_id = iv.invoice_id
                 LEFT JOIN reduced f ON f.tenant_id = iv.tenant_id AND f.invoice_id = iv.invoice_id),
             -- `InvoiceCollections::uncollected`, in SQL.
             backing AS (
                 SELECT m.tenant_id, m.invoice_id, m.since, m.touched_at,
                        greatest(least(m.reversed + m.refunded,
                                       m.owed - (m.gross - m.reversed - m.excess_refunded - m.refunded
                                                 - m.owed_back)),
                                 0) AS unbacked,
                        greatest(m.owed, 0) AS standing
                 FROM money m),
             per_grant AS (
                 SELECT pu.tenant_id, pu.invoice_id, pu.account_id, pu.remaining, pu.classification,
                        coalesce(m.clawed, 0) AS clawed,
                        pu.amount - coalesce(m.by_notes, 0) AS capacity,
                        b.unbacked, b.standing
                 FROM purchase pu
                 JOIN backing b ON b.tenant_id = pu.tenant_id AND b.invoice_id = pu.invoice_id
                 LEFT JOIN moved m ON m.tenant_id = pu.tenant_id AND m.invoice_id = pu.invoice_id
                                  AND m.grant_id = pu.grant_id),
             wanted AS (
                 SELECT g.*,
                        (CASE WHEN g.unbacked <= 0 OR g.standing <= 0 OR g.capacity <= 0 THEN 0
                              ELSE least(g.capacity,
                                         ceil(g.capacity::numeric * least(g.unbacked, g.standing) / g.standing))
                         END)::bigint AS want
                 FROM per_grant g),
             wallets AS (
                 SELECT tenant_id, invoice_id, account_id,
                        bool_or(clawed > want OR (clawed < want AND remaining > 0)) AS ledger_out,
                        bool_or(clawed <> 0) AS clawed_any,
                        -- Bonus credit sold with the purchase was never paid for: no loss.
                        sum(CASE WHEN classification = 'PROMOTIONAL' THEN 0
                                 ELSE greatest(want - clawed, 0) END)::bigint AS short
                 FROM wanted
                 GROUP BY tenant_id, invoice_id, account_id),
             flagged AS (
                 SELECT w.tenant_id, w.invoice_id,
                        bool_or(w.ledger_out OR w.short <> coalesce(rp.reported, 0)) AS out_of_sync,
                        bool_or(w.clawed_any OR coalesce(rp.reported, 0) <> 0) AS engaged
                 FROM wallets w
                 LEFT JOIN reported rp ON rp.tenant_id = w.tenant_id AND rp.invoice_id = w.invoice_id
                                      AND rp.account_id = w.account_id
                 GROUP BY w.tenant_id, w.invoice_id)
             SELECT k.tenant_id, k.invoice_id,
                    coalesce(f.out_of_sync AND (b.touched_at > b.since OR f.engaged), false) AS out_of_sync
             FROM keys k
             LEFT JOIN flagged f ON f.tenant_id = k.tenant_id AND f.invoice_id = k.invoice_id
             LEFT JOIN backing b ON b.tenant_id = k.tenant_id AND b.invoice_id = k.invoice_id
             ORDER BY k.tenant_id, k.invoice_id",
        )
        .bind::<SqlUuid, _>(after.0.as_uuid())
        .bind::<SqlUuid, _>(after.1.as_uuid())
        .bind::<BigInt, _>(limit)
        .load(conn)
        .await
        .attach("Error while listing purchase invoices with unreconciled reversals")
        .into_db_result()
    }
}

impl UnreversedRefundNote {
    /// Refund notes finalized after a purchase grant existed that hit its line and left no
    /// reversal on it, after the `(tenant, note)` cursor. A grant with nothing left to reverse
    /// is not listed.
    pub async fn list(
        conn: &mut PgConn,
        after: (TenantId, CreditNoteId),
        limit: i64,
    ) -> DbResult<Vec<Self>> {
        diesel::sql_query(
            "SELECT cn.tenant_id, cn.id AS credit_note_id
             FROM credit_note cn
             JOIN customer_credit_grant g
               ON g.tenant_id = cn.tenant_id
              AND g.invoice_id = cn.invoice_id
              AND g.source_kind = 'CREDIT_PURCHASE'
              AND g.line_local_id IS NOT NULL
              AND g.remaining > 0
             LEFT JOIN customer_credit_entry e
               ON e.tenant_id = g.tenant_id AND e.grant_id = g.id
              AND e.credit_note_id = cn.id AND e.kind = 'REVERSAL'
             WHERE cn.status = 'FINALIZED'
               AND cn.credit_type = 'REFUND'
               AND cn.finalized_at > g.created_at
               AND (cn.tenant_id, cn.id) > ($1, $2)
               AND e.id IS NULL
               AND jsonb_path_exists(
                       cn.line_items,
                       '$[*] ? (@.local_id == $local_id && (@.amount_subtotal != 0 || @.subtotal != 0))',
                       jsonb_build_object('local_id', g.line_local_id))
             GROUP BY cn.tenant_id, cn.id
             ORDER BY cn.tenant_id, cn.id
             LIMIT $3",
        )
        .bind::<SqlUuid, _>(after.0.as_uuid())
        .bind::<SqlUuid, _>(after.1.as_uuid())
        .bind::<BigInt, _>(limit)
        .load(conn)
        .await
        .attach("Error while listing refund credit notes without a credit reversal")
        .into_db_result()
    }
}

/// A payment kept whole as credit (a grant no invoice owns) that may be out of line with its money.
#[derive(Debug, Clone, Copy, diesel::QueryableByName)]
pub struct KeptPaymentOutOfLine {
    #[diesel(sql_type = SqlUuid)]
    pub tenant_id: TenantId,
    #[diesel(sql_type = SqlUuid)]
    pub payment_id: PaymentTransactionId,
    #[diesel(sql_type = SqlUuid)]
    pub account_id: WalletId,
    /// The credit differs from the target `follow_collected_payment_tx` derives (what was reversed
    /// or refunded without a note, up to the credit kept).
    #[diesel(sql_type = Bool)]
    pub ledger_out: bool,
    /// The spent credit that target leaves unrecovered, once the credit is in line.
    #[diesel(sql_type = BigInt)]
    pub unrecovered: i64,
}

impl KeptPaymentOutOfLine {
    /// The next `limit` such payments after the `(tenant, payment)` cursor: out of line, or with a
    /// loss to report or reported. The report is keyed by a hash of the payment SQL cannot derive,
    /// so the caller compares it.
    pub async fn list(
        conn: &mut PgConn,
        after: (TenantId, PaymentTransactionId),
        limit: i64,
    ) -> DbResult<Vec<Self>> {
        diesel::sql_query(
            "WITH kept AS (
                 SELECT g.tenant_id, g.source_id AS payment_id, g.id AS grant_id, g.account_id,
                        g.amount, g.remaining
                 FROM customer_credit_grant g
                 WHERE g.source_kind = 'OVERPAYMENT'
                   AND g.invoice_id IS NULL
                   AND (g.tenant_id, g.source_id) > ($1, $2)),
             refunded AS (
                 SELECT r.tenant_id, r.parent_transaction_id AS payment_id,
                        sum(r.amount)::bigint AS refunded
                 FROM payment_transaction r
                 JOIN kept k ON k.tenant_id = r.tenant_id AND k.payment_id = r.parent_transaction_id
                 WHERE r.payment_type = 'REFUND'
                   AND r.credit_note_id IS NULL
                   AND r.status IN ('READY', 'PENDING', 'SETTLED')
                 GROUP BY r.tenant_id, r.parent_transaction_id),
             moved AS (
                 SELECT e.grant_id, -sum(e.amount)::bigint AS taken
                 FROM customer_credit_entry e
                 JOIN kept k ON k.tenant_id = e.tenant_id AND k.grant_id = e.grant_id
                 WHERE e.kind IN ('REVERSAL', 'REINSTATEMENT')
                   AND e.credit_note_id IS NULL
                 GROUP BY e.grant_id),
             wanted AS (
                 SELECT k.tenant_id, k.payment_id, k.account_id, k.amount, k.remaining,
                        p.invoice_id,
                        least(greatest(p.amount_reversed + coalesce(rf.refunded, 0), 0), k.amount)
                            AS target,
                        coalesce(m.taken, 0) AS taken
                 FROM kept k
                 JOIN payment_transaction p ON p.tenant_id = k.tenant_id AND p.id = k.payment_id
                 LEFT JOIN refunded rf ON rf.tenant_id = k.tenant_id AND rf.payment_id = k.payment_id
                 LEFT JOIN moved m ON m.grant_id = k.grant_id),
             reports AS (
                 SELECT DISTINCT w.tenant_id, w.payment_id
                 FROM wanted w
                 JOIN customer_credit_unrecovered u
                   ON u.tenant_id = w.tenant_id AND u.account_id = w.account_id
                  AND (u.payment_id = w.payment_id OR u.invoice_id = w.invoice_id)
                 WHERE u.reported <> 0)
             SELECT w.tenant_id, w.payment_id, w.account_id,
                    ((w.target > w.taken AND w.remaining > 0)
                     OR (w.target < w.taken AND w.remaining < w.amount)) AS ledger_out,
                    greatest(w.target - w.taken, 0)::bigint AS unrecovered
             FROM wanted w
             LEFT JOIN reports r ON r.tenant_id = w.tenant_id AND r.payment_id = w.payment_id
             WHERE (w.target > w.taken AND w.remaining > 0)
                OR (w.target < w.taken AND w.remaining < w.amount)
                OR w.target > w.taken
                OR r.payment_id IS NOT NULL
             ORDER BY w.tenant_id, w.payment_id
             LIMIT $3",
        )
        .bind::<SqlUuid, _>(after.0.as_uuid())
        .bind::<SqlUuid, _>(after.1.as_uuid())
        .bind::<BigInt, _>(limit)
        .load(conn)
        .await
        .attach("Error while listing payments kept as credit out of line with their money")
        .into_db_result()
    }
}

/// An invoice with money lost to a dispute that is no longer marked paid: a previous release's
/// recompute reopened it while both ran.
#[derive(Debug, Clone, Copy, diesel::QueryableByName)]
pub struct DisputedUnpaidInvoice {
    #[diesel(sql_type = SqlUuid)]
    pub tenant_id: TenantId,
    #[diesel(sql_type = SqlUuid)]
    pub invoice_id: InvoiceId,
}

impl DisputedUnpaidInvoice {
    /// The next `limit` such invoices after the `(tenant, invoice)` cursor.
    pub async fn list(
        conn: &mut PgConn,
        after: (TenantId, InvoiceId),
        limit: i64,
    ) -> DbResult<Vec<Self>> {
        diesel::sql_query(
            "SELECT DISTINCT m.tenant_id, m.invoice_id
             FROM payment_reversal_movement m
             JOIN invoice i ON i.tenant_id = m.tenant_id AND i.id = m.invoice_id
             WHERE m.disputed
               AND m.invoice_id IS NOT NULL
               AND (m.tenant_id, m.invoice_id) > ($1, $2)
               AND i.payment_disputed_at IS NOT NULL
               AND i.payment_status <> 'PAID'
             ORDER BY m.tenant_id, m.invoice_id
             LIMIT $3",
        )
        .bind::<SqlUuid, _>(after.0.as_uuid())
        .bind::<SqlUuid, _>(after.1.as_uuid())
        .bind::<BigInt, _>(limit)
        .load(conn)
        .await
        .attach("Error while listing disputed invoices no longer paid")
        .into_db_result()
    }
}
