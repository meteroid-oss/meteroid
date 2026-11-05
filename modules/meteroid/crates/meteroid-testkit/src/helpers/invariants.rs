//! The credit billing invariants that the database alone can
//! prove: the ledger, invoices, credit notes, usage settlements and usage filters agree with each
//! other. Every test's database is checked when it is dropped (`TestDatabase`), so a test gets
//! them without asking; `assert_credit_invariants` checks one tenant mid-test.
use common_domain::ids::{BaseId, TenantId};
use diesel::sql_types::{Array, Text};
use diesel_async::{RunQueryDsl, SimpleAsyncConnection};
use meteroid_store::Store;
use meteroid_store::store::PgConn;
use uuid::Uuid;

#[derive(diesel::QueryableByName)]
struct Violation {
    #[diesel(sql_type = Text)]
    message: String,
}

#[derive(diesel::QueryableByName)]
struct IdRow {
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    id: Uuid,
}

/// At most this many violations are reported per invariant.
const SHOWN: usize = 20;

/// Each invariant: a name and a query over tenant `$1` returning one `message` per violation.
/// `$2`/`$3` are the tenant's invoices and the source of their usage settlement excess returns,
/// `$4`/`$5` its settlements and the source of their give-backs (computed in Rust, as UUIDv5).
const CHECKS: &[(&str, &str)] = &[
    (
        "I1 balance = sum of movements",
        "SELECT format('grant %s holds %s, its movements sum to %s', g.id, g.remaining,
                       coalesce(m.total, 0)) AS message
         FROM customer_credit_grant g
         LEFT JOIN (SELECT grant_id, sum(amount) AS total FROM customer_credit_entry
                    WHERE tenant_id = $1 GROUP BY grant_id) m ON m.grant_id = g.id
         WHERE g.tenant_id = $1 AND g.remaining <> coalesce(m.total, 0)",
    ),
    (
        "I1 one funding of the grant's face",
        "SELECT format('grant %s of %s has %s FUND movement(s) totalling %s', g.id, g.amount,
                       coalesce(f.n, 0), f.total) AS message
         FROM customer_credit_grant g
         LEFT JOIN (SELECT grant_id, count(*) AS n, sum(amount) AS total FROM customer_credit_entry
                    WHERE tenant_id = $1 AND kind = 'FUND' GROUP BY grant_id) f ON f.grant_id = g.id
         WHERE g.tenant_id = $1 AND (f.n IS DISTINCT FROM 1 OR f.total <> g.amount)",
    ),
    (
        "I1 running balance within [0, face] in business-time order",
        "WITH running AS (
             SELECT e.grant_id, e.id, e.kind, e.occurred_at,
                    sum(e.amount) OVER (PARTITION BY e.grant_id
                                        ORDER BY e.occurred_at, e.created_at, e.id
                                        ROWS UNBOUNDED PRECEDING) AS balance
             FROM customer_credit_entry e WHERE e.tenant_id = $1)
         SELECT format('grant %s of %s runs to %s after its %s %s at %s', g.id, g.amount,
                       r.balance, r.kind, r.id, r.occurred_at) AS message
         FROM running r JOIN customer_credit_grant g ON g.id = r.grant_id
         WHERE r.balance < 0 OR r.balance > g.amount",
    ),
    (
        "I1 movements carry the grant's cost exactly",
        "SELECT format('grant %s cost %s; its movements carry %s, the %s it holds should carry %s',
                       g.id, g.cost_minor, coalesce(c.carried, 0), g.remaining,
                       g.cost_minor - div(2 * (g.amount - g.remaining)::numeric * g.cost_minor
                                          + g.amount, 2 * g.amount::numeric)) AS message
         FROM customer_credit_grant g
         LEFT JOIN (SELECT grant_id, sum(coalesce(cost_minor, 0)) AS carried
                    FROM customer_credit_entry WHERE tenant_id = $1 GROUP BY grant_id) c
                ON c.grant_id = g.id
         WHERE g.tenant_id = $1 AND g.cost_minor IS NOT NULL
           AND coalesce(c.carried, 0) <> g.cost_minor
                 - div(2 * (g.amount - g.remaining)::numeric * g.cost_minor + g.amount,
                       2 * g.amount::numeric)",
    ),
    (
        "I7 paid currency credit costs its face, promotional credit nothing",
        "SELECT format('%s grant %s of %s %s costs %s %s', g.classification, g.id, g.amount,
                       a.currency, g.cost_minor, g.cost_currency) AS message
         FROM customer_credit_grant g JOIN customer_credit_account a ON a.id = g.account_id
         WHERE g.tenant_id = $1 AND a.credit_unit_id IS NULL
           AND ((g.classification = 'PAID'
                 AND (g.cost_minor IS DISTINCT FROM g.amount
                      OR g.cost_currency IS DISTINCT FROM a.currency))
                OR (g.classification = 'PROMOTIONAL' AND coalesce(g.cost_minor, 0) <> 0))",
    ),
    (
        "I7 bonus credit has a paid twin with its terms",
        "WITH products AS (
             SELECT grant_id, array_agg(product_id ORDER BY product_id) AS ids
             FROM customer_credit_grant_product WHERE tenant_id = $1 GROUP BY grant_id),
         twins AS (
             SELECT DISTINCT b.id
             FROM customer_credit_grant b
             JOIN customer_credit_grant p
               ON p.account_id = b.account_id AND p.classification = 'PAID'
              AND p.source_kind = b.source_kind
              AND p.invoice_id IS NOT DISTINCT FROM b.invoice_id
              AND p.line_local_id IS NOT DISTINCT FROM b.line_local_id
              AND p.subscription_id IS NOT DISTINCT FROM b.subscription_id
              AND p.application = b.application AND p.effective_at = b.effective_at
              AND p.expires_at IS NOT DISTINCT FROM b.expires_at
             LEFT JOIN products bp ON bp.grant_id = b.id
             LEFT JOIN products pp ON pp.grant_id = p.id
             WHERE b.tenant_id = $1 AND bp.ids IS NOT DISTINCT FROM pp.ids)
         SELECT format('bonus grant %s of %s (%s) has no paid twin with the same terms',
                       b.id, b.amount, b.source_kind) AS message
         FROM customer_credit_grant b LEFT JOIN twins t ON t.id = b.id
         WHERE b.tenant_id = $1 AND b.classification = 'PROMOTIONAL' AND b.reason = 'Bonus credit'
           AND t.id IS NULL",
    ),
    (
        "I8 plan credit pays its own subscription's usage only",
        "WITH products AS (
             SELECT grant_id, count(*) AS n FROM customer_credit_grant_product
             WHERE tenant_id = $1 GROUP BY grant_id)
         SELECT format('%s grant %s: subscription %s, application %s, %s product(s)',
                       g.source_kind, g.id, g.subscription_id, g.application,
                       coalesce(p.n, 0)) AS message
         FROM customer_credit_grant g
         JOIN customer_credit_account a ON a.id = g.account_id
         LEFT JOIN products p ON p.grant_id = g.id
         WHERE g.tenant_id = $1 AND a.credit_unit_id IS NULL
           AND (g.source_kind = 'INCLUDED_CREDIT'
                OR (g.source_kind = 'CREDIT_PURCHASE' AND g.subscription_id IS NOT NULL
                    AND g.invoice_id IN (SELECT id FROM invoice WHERE tenant_id = $1
                                         AND invoice_type <> 'ONE_OFF')))
           AND (g.subscription_id IS NULL OR g.application <> 'USAGE' OR p.n IS NOT NULL)",
    ),
    (
        "R7 credit notes, voids, negative invoices and overpayments are customer-wide",
        "WITH products AS (
             SELECT grant_id, count(*) AS n FROM customer_credit_grant_product
             WHERE tenant_id = $1 GROUP BY grant_id)
         SELECT format('%s grant %s: subscription %s, application %s, %s product(s)',
                       g.source_kind, g.id, g.subscription_id, g.application,
                       coalesce(p.n, 0)) AS message
         FROM customer_credit_grant g LEFT JOIN products p ON p.grant_id = g.id
         WHERE g.tenant_id = $1
           AND g.source_kind IN ('CREDIT_NOTE', 'NEGATIVE_INVOICE', 'INVOICE_VOID_RESTORE',
                                 'OVERPAYMENT')
           AND (g.subscription_id IS NOT NULL OR g.application <> 'INVOICE' OR p.n IS NOT NULL)",
    ),
    (
        "I8 subscription credit is never drawn for another subscription",
        "WITH draws AS (
             SELECT e.id, e.grant_id, coalesce(e.invoice_id, s.invoice_id) AS invoice_id,
                    s.subscription_id AS settled_for
             FROM customer_credit_entry e
             LEFT JOIN credit_usage_settlement s
                    ON e.source_kind = 'USAGE_SETTLEMENT' AND s.id = e.source_id
             WHERE e.tenant_id = $1 AND e.kind = 'DRAW'),
         billed AS (
             SELECT id AS invoice_id, subscription_id FROM invoice
             WHERE tenant_id = $1 AND subscription_id IS NOT NULL
             UNION
             SELECT consolidated_into_invoice_id, subscription_id FROM invoice
             WHERE tenant_id = $1 AND consolidated_into_invoice_id IS NOT NULL
               AND subscription_id IS NOT NULL
             UNION
             SELECT i.id, coalesce((l->>'subscription_id')::uuid, sc.subscription_id,
                                   sa.subscription_id)
             FROM invoice i
             CROSS JOIN LATERAL jsonb_array_elements(i.line_items) l
             LEFT JOIN subscription_component sc ON sc.id = (l->>'sub_component_id')::uuid
             LEFT JOIN subscription_add_on sa ON sa.id = (l->>'sub_add_on_id')::uuid
             WHERE i.tenant_id = $1)
         SELECT format('draw %s of grant %s (subscription %s) pays invoice %s / settlement of %s',
                       d.id, g.id, g.subscription_id, d.invoice_id, d.settled_for) AS message
         FROM draws d JOIN customer_credit_grant g ON g.id = d.grant_id
         WHERE g.subscription_id IS NOT NULL
           AND ((d.settled_for IS NOT NULL AND d.settled_for <> g.subscription_id)
                OR (d.invoice_id IS NOT NULL AND NOT EXISTS (
                        SELECT 1 FROM billed b
                        WHERE b.invoice_id = d.invoice_id
                          AND b.subscription_id = g.subscription_id)))",
    ),
    (
        "I5 an invoice never draws credit that lapsed",
        "SELECT format('grant %s lapsing at %s is drawn by invoice %s at %s', g.id, g.lapses_at,
                       e.invoice_id, e.occurred_at) AS message
         FROM customer_credit_entry e JOIN customer_credit_grant g ON g.id = e.grant_id
         WHERE e.tenant_id = $1 AND e.kind = 'DRAW' AND e.source_kind = 'INVOICE'
           AND g.lapses_at IS NOT NULL AND e.occurred_at >= g.lapses_at",
    ),
    (
        "I5 credit pays only the lines it may pay, usage only when effective before it ended",
        "WITH lines AS (
             SELECT i.id AS invoice_id, l,
                    coalesce((l->>'amount_total')::bigint, (l->>'total')::bigint, 0)
                      + coalesce((l->>'promotional_credit')::bigint, 0) AS covered,
                    l->>'metric_id' AS metric_id, l->>'product_id' AS product_id,
                    coalesce(l->>'kind', 'Regular') AS kind,
                    coalesce((l->>'usage_ends_at')::timestamptz,
                             ((l->>'end_date')::date)::timestamp AT TIME ZONE 'UTC') AS ends_at
             FROM invoice i, jsonb_array_elements(i.line_items) l WHERE i.tenant_id = $1),
         held AS (
             SELECT coalesce(e.invoice_id, s.invoice_id) AS invoice_id, e.grant_id,
                    -sum(e.amount) AS amount
             FROM customer_credit_entry e
             JOIN customer_credit_grant g ON g.id = e.grant_id
             JOIN customer_credit_account a ON a.id = g.account_id AND a.credit_unit_id IS NULL
             LEFT JOIN credit_usage_settlement s
                    ON e.source_kind = 'USAGE_SETTLEMENT' AND s.id = e.source_id
             WHERE e.tenant_id = $1 AND e.kind IN ('DRAW', 'DRAW_REVERSAL')
             GROUP BY 1, 2),
         products AS (
             SELECT grant_id, array_agg(product_id::text) AS ids
             FROM customer_credit_grant_product WHERE tenant_id = $1 GROUP BY grant_id),
         payable AS (
             SELECT h.invoice_id, h.grant_id, h.amount,
                    coalesce(sum(l.covered) FILTER (
                        WHERE l.kind <> 'Credit'
                          AND (g.application <> 'USAGE' OR l.metric_id IS NOT NULL)
                          AND (p.ids IS NULL OR l.product_id = ANY (p.ids))
                          -- usage only by credit whose usage start is before its line ended
                          -- (value main kept pays any)
                          AND (l.metric_id IS NULL OR l.kind <> 'Regular'
                               OR g.source_kind IN ('CUSTOMER_BALANCE', 'INVOICE_VOID_RESTORE')
                               OR coalesce(g.pays_usage_from, g.effective_at) < l.ends_at)), 0)
                      AS limit_minor
             FROM held h JOIN customer_credit_grant g ON g.id = h.grant_id
             LEFT JOIN products p ON p.grant_id = h.grant_id
             LEFT JOIN lines l ON l.invoice_id = h.invoice_id
             WHERE h.invoice_id IS NOT NULL
               AND h.invoice_id NOT IN (SELECT id FROM invoice WHERE tenant_id = $1 AND status = 'VOID')
             GROUP BY h.invoice_id, h.grant_id, h.amount)
         SELECT format('grant %s pays %s of invoice %s, whose lines it may pay come to %s',
                       grant_id, amount, invoice_id, limit_minor) AS message
         FROM payable WHERE amount > limit_minor",
    ),
    (
        "I3 an invoice's credit is what the ledger drew for it",
        "WITH excess AS (SELECT * FROM unnest($2::uuid[], $3::uuid[]) AS x(invoice_id, source_id)),
         moves AS (
             SELECT e.invoice_id, e.grant_id, e.amount
             FROM customer_credit_entry e LEFT JOIN excess x ON x.invoice_id = e.invoice_id
             WHERE e.tenant_id = $1 AND e.invoice_id IS NOT NULL AND e.credit_note_id IS NULL
               AND (e.kind = 'DRAW' OR (e.kind = 'DRAW_REVERSAL' AND e.source_id = x.source_id))
             UNION ALL
             SELECT s.invoice_id, e.grant_id, e.amount
             FROM customer_credit_entry e
             JOIN credit_usage_settlement s ON s.id = e.source_id
             WHERE e.tenant_id = $1 AND e.source_kind = 'USAGE_SETTLEMENT' AND e.kind = 'DRAW'
               AND e.invoice_id IS NULL AND s.invoice_id IS NOT NULL),
         drawn AS (
             SELECT m.invoice_id,
                    -sum(m.amount) FILTER (WHERE g.classification = 'PROMOTIONAL') AS promo,
                    -sum(m.amount) FILTER (WHERE g.classification = 'PAID') AS paid
             FROM moves m
             JOIN customer_credit_grant g ON g.id = m.grant_id
             JOIN customer_credit_account a ON a.id = g.account_id
             JOIN invoice i ON i.id = m.invoice_id AND a.currency = upper(i.currency)
             GROUP BY m.invoice_id),
         legacy AS (
             SELECT invoice_id, -sum(amount_cents) AS paid FROM customer_balance_tx
             WHERE tenant_id = $1 AND invoice_id IS NOT NULL AND amount_cents < 0
             GROUP BY invoice_id),
         shown AS (
             SELECT i.id,
                    CASE WHEN i.tax_inclusive
                         THEN (SELECT coalesce(sum((l->>'promotional_credit')::bigint), 0)
                               FROM jsonb_array_elements(i.line_items) l)
                         ELSE i.promotional_credits END AS promo
             FROM invoice i WHERE i.tenant_id = $1)
         SELECT format('%s invoice %s shows %s promotional and %s applied credit, the ledger drew %s and %s (+%s before the ledger)',
                       i.status, i.id, sh.promo, i.applied_credits, coalesce(d.promo, 0),
                       coalesce(d.paid, 0), coalesce(lg.paid, 0)) AS message
         FROM invoice i
         JOIN shown sh ON sh.id = i.id
         LEFT JOIN drawn d ON d.invoice_id = i.id
         LEFT JOIN legacy lg ON lg.invoice_id = i.id
         WHERE i.tenant_id = $1 AND i.status IN ('FINALIZED', 'VOID', 'UNCOLLECTIBLE')
           AND (coalesce(d.promo, 0) <> sh.promo
                OR coalesce(d.paid, 0) + coalesce(lg.paid, 0) <> i.applied_credits)",
    ),
    (
        "I3 a draft draws no credit",
        "SELECT format('draft invoice %s has a %s of %s on grant %s', i.id, e.kind, e.amount,
                       e.grant_id) AS message
         FROM customer_credit_entry e JOIN invoice i ON i.id = e.invoice_id
         WHERE e.tenant_id = $1 AND i.status = 'DRAFT' AND e.kind IN ('DRAW', 'DRAW_REVERSAL')",
    ),
    (
        "I3 credit given back never exceeds what was drawn",
        "SELECT format('invoice %s gave grant %s back %s more than it drew', x.invoice_id,
                       x.grant_id, x.net) AS message
         FROM (SELECT coalesce(e.invoice_id, s.invoice_id) AS invoice_id, e.grant_id,
                      sum(e.amount) AS net
               FROM customer_credit_entry e
               LEFT JOIN credit_usage_settlement s
                      ON e.source_kind = 'USAGE_SETTLEMENT' AND s.id = e.source_id
               WHERE e.tenant_id = $1 AND e.kind IN ('DRAW', 'DRAW_REVERSAL')
               GROUP BY 1, 2) x
         WHERE x.invoice_id IS NOT NULL AND x.net > 0",
    ),
    (
        "I3 a void returns every draw",
        "SELECT format('void invoice %s still holds %s of grant %s', x.invoice_id, -x.net,
                       x.grant_id) AS message
         FROM (SELECT coalesce(e.invoice_id, s.invoice_id) AS invoice_id, e.grant_id,
                      sum(e.amount) AS net
               FROM customer_credit_entry e
               LEFT JOIN credit_usage_settlement s
                      ON e.source_kind = 'USAGE_SETTLEMENT' AND s.id = e.source_id
               WHERE e.tenant_id = $1 AND e.kind IN ('DRAW', 'DRAW_REVERSAL')
               GROUP BY 1, 2) x
         JOIN invoice i ON i.id = x.invoice_id AND i.status = 'VOID'
         WHERE x.net <> 0",
    ),
    (
        "I2 invoice identity",
        "WITH sums AS (
             SELECT i.id,
                    coalesce(sum(coalesce((l->>'amount_subtotal')::bigint,
                                          (l->>'subtotal')::bigint)), 0) AS subtotal,
                    coalesce(sum((l->>'tax_amount')::bigint), 0) AS tax,
                    coalesce(sum((l->>'promotional_credit')::bigint), 0) AS promo
             FROM invoice i LEFT JOIN LATERAL jsonb_array_elements(i.line_items) l ON true
             WHERE i.tenant_id = $1 GROUP BY i.id)
         SELECT format('%s invoice %s: subtotal %s (lines %s), discount %s, promotional %s (lines %s), tax %s (lines %s), total %s, applied %s, due %s',
                       i.status, i.id, i.subtotal, s.subtotal, i.discount,
                       i.promotional_credits, s.promo, i.tax_amount, s.tax, i.total,
                       i.applied_credits, i.amount_due) AS message
         FROM invoice i JOIN sums s ON s.id = i.id
         WHERE i.tenant_id = $1 AND i.status NOT IN ('DRAFT', 'CLOSED')
           AND (i.subtotal <> s.subtotal OR i.tax_amount <> s.tax
                OR (NOT i.tax_inclusive AND i.promotional_credits <> s.promo)
                OR i.total <> i.subtotal - i.discount - i.promotional_credits + i.tax_amount
                OR i.amount_due < 0 OR i.applied_credits < 0 OR i.promotional_credits < 0)",
    ),
    (
        "I2 applied credit and amount due never exceed the total",
        "SELECT format('%s invoice %s: total %s, applied %s, due %s', i.status, i.id, i.total,
                       i.applied_credits, i.amount_due) AS message
         FROM invoice i
         WHERE i.tenant_id = $1 AND i.status IN ('FINALIZED', 'UNCOLLECTIBLE')
           AND (i.amount_due > greatest(0, i.total - i.applied_credits)
                OR i.applied_credits > greatest(0, i.total))",
    ),
    // R26 (owing nothing ⇒ paid) is reached by the orchestration that follows the PDF, which a
    // test need not run; scenarios assert it after running it.
    (
        "A paid invoice owes nothing",
        "SELECT format('paid invoice %s owes %s of %s', i.id, i.amount_due, i.total) AS message
         FROM invoice i
         WHERE i.tenant_id = $1 AND i.status = 'FINALIZED' AND i.payment_status = 'PAID'
           AND i.amount_due <> 0",
    ),
    (
        "Credit notes never credit more than their invoice line",
        "WITH billed AS (
             SELECT i.id AS invoice_id, l->>'local_id' AS local_id,
                    abs(coalesce((l->>'amount_subtotal')::bigint, (l->>'subtotal')::bigint))
                      AS subtotal,
                    abs(coalesce((l->>'tax_amount')::bigint, 0)) AS tax
             FROM invoice i, jsonb_array_elements(i.line_items) l WHERE i.tenant_id = $1),
         credited AS (
             SELECT n.invoice_id, l->>'local_id' AS local_id,
                    sum(abs(coalesce((l->>'amount_subtotal')::bigint, (l->>'subtotal')::bigint)))
                      AS subtotal,
                    sum(abs(coalesce((l->>'tax_amount')::bigint, 0))) AS tax
             FROM credit_note n, jsonb_array_elements(n.line_items) l
             WHERE n.tenant_id = $1 AND n.status <> 'VOIDED'
             GROUP BY 1, 2)
         SELECT format('notes on invoice %s credit line %s %s (tax %s) of %s (tax %s)',
                       c.invoice_id, c.local_id, c.subtotal, c.tax, b.subtotal, b.tax) AS message
         FROM credited c
         LEFT JOIN billed b ON b.invoice_id = c.invoice_id AND b.local_id = c.local_id
         WHERE b.local_id IS NULL OR c.subtotal > b.subtotal OR c.tax > b.tax",
    ),
    (
        "Credit notes never exceed their invoice",
        "SELECT format('notes on invoice %s total %s of its %s', i.id, n.total, i.total) AS message
         FROM invoice i
         JOIN (SELECT invoice_id, sum(abs(total)) AS total FROM credit_note
               WHERE tenant_id = $1 AND status <> 'VOIDED' GROUP BY invoice_id) n
           ON n.invoice_id = i.id
         WHERE i.tenant_id = $1 AND n.total > abs(i.total)",
    ),
    (
        "Credit note identity",
        "WITH sums AS (
             SELECT n.id,
                    coalesce(sum(coalesce((l->>'amount_subtotal')::bigint,
                                          (l->>'subtotal')::bigint)), 0) AS subtotal,
                    coalesce(sum((l->>'tax_amount')::bigint), 0) AS tax,
                    coalesce(sum(coalesce((l->>'amount_total')::bigint, (l->>'total')::bigint)), 0)
                      AS total
             FROM credit_note n LEFT JOIN LATERAL jsonb_array_elements(n.line_items) l ON true
             WHERE n.tenant_id = $1 GROUP BY n.id)
         SELECT format('%s note %s: subtotal %s (lines %s), tax %s (lines %s), total %s (lines %s); refunded %s, credited %s, debt cancelled %s',
                       n.status, n.id, n.subtotal, s.subtotal, n.tax_amount, s.tax, n.total,
                       s.total, n.refunded_amount_cents, n.credited_amount_cents,
                       n.debt_cancelled_amount_cents) AS message
         FROM credit_note n JOIN sums s ON s.id = n.id
         WHERE n.tenant_id = $1
           AND (n.subtotal <> s.subtotal OR n.tax_amount <> s.tax OR n.total <> s.total
                OR (n.status = 'FINALIZED' AND NOT n.voids_invoice
                    AND n.refunded_amount_cents + n.credited_amount_cents
                        + n.debt_cancelled_amount_cents <> abs(n.total)))",
    ),
    (
        "Settlement = its draws",
        "SELECT format('settlement %s of %s: its draws total %s', s.id, s.amount,
                       coalesce(d.total, 0)) AS message
         FROM credit_usage_settlement s
         LEFT JOIN (SELECT source_id, -sum(amount) AS total FROM customer_credit_entry
                    WHERE tenant_id = $1 AND kind = 'DRAW' AND source_kind = 'USAGE_SETTLEMENT'
                    GROUP BY source_id) d ON d.source_id = s.id
         WHERE s.tenant_id = $1 AND s.amount <> coalesce(d.total, 0)",
    ),
    (
        "Settlement returned whole when no invoice took it",
        "WITH give_back AS (SELECT * FROM unnest($4::uuid[], $5::uuid[]) AS x(settlement_id, source_id))
         SELECT format('returned settlement %s of %s came back %s', s.id, s.amount,
                       coalesce(r.total, 0)) AS message
         FROM credit_usage_settlement s
         JOIN give_back gb ON gb.settlement_id = s.id
         LEFT JOIN (SELECT source_id, sum(amount) AS total FROM customer_credit_entry
                    WHERE tenant_id = $1 AND kind = 'DRAW_REVERSAL' GROUP BY source_id) r
                ON r.source_id = gb.source_id
         WHERE s.tenant_id = $1 AND s.returned_at IS NOT NULL
           AND s.amount <> coalesce(r.total, 0)",
    ),
    (
        "Settlement taken over by one issued invoice of its subscription",
        "WITH billed AS (
             SELECT id AS invoice_id, subscription_id, customer_id, status FROM invoice
             WHERE tenant_id = $1
             UNION ALL
             SELECT parent.id, child.subscription_id, parent.customer_id, parent.status
             FROM invoice child JOIN invoice parent ON parent.id = child.consolidated_into_invoice_id
             WHERE child.tenant_id = $1)
         SELECT format('settlement %s of subscription %s is taken over by %s invoice %s',
                       s.id, s.subscription_id, coalesce(b.status::text, 'a missing'),
                       s.invoice_id) AS message
         FROM credit_usage_settlement s
         LEFT JOIN billed b ON b.invoice_id = s.invoice_id AND b.subscription_id = s.subscription_id
                           AND b.customer_id = s.customer_id
         WHERE s.tenant_id = $1 AND s.invoice_id IS NOT NULL
           AND (b.invoice_id IS NULL OR b.status = 'DRAFT')",
    ),
    (
        "A period's settlements are taken over by at most one invoice",
        "SELECT format('the settlements of subscription %s for %s (wallet %s) went to %s invoices',
                       subscription_id, period_start, account_id,
                       count(DISTINCT invoice_id)) AS message
         FROM credit_usage_settlement
         WHERE tenant_id = $1 AND invoice_id IS NOT NULL
         GROUP BY subscription_id, period_start, account_id
         HAVING count(DISTINCT invoice_id) > 1",
    ),
    (
        "I9 credit ends with its subscription",
        "SELECT format('grant %s of %s subscription %s has no end (expires %s)', g.id, s.status,
                       s.id, g.expires_at) AS message
         FROM customer_credit_grant g
         JOIN subscription s ON s.id = g.subscription_id AND s.tenant_id = g.tenant_id
         WHERE g.tenant_id = $1 AND g.ends_at IS NULL
           AND s.status IN ('CANCELLED', 'COMPLETED', 'ABORTED', 'SUPERSEDED')
           AND (g.expires_at IS NULL
                OR g.expires_at >= LEAST(COALESCE(s.terminated_at, s.current_period_start::timestamp)
                                           AT TIME ZONE 'UTC', now()))",
    ),
    (
        "I13 rollover",
        "WITH products AS (
             SELECT grant_id, array_agg(product_id ORDER BY product_id) AS ids
             FROM customer_credit_grant_product WHERE tenant_id = $1 GROUP BY grant_id),
         rolled AS (
             SELECT grant_id, count(*) AS n, -sum(amount) AS carried FROM customer_credit_entry
             WHERE tenant_id = $1 AND kind = 'ROLLOVER' GROUP BY grant_id)
         SELECT format('rollover grant %s of %s from %s: carried %s (x%s), classification %s/%s, subscription %s/%s, application %s/%s, priority %s/%s, from %s/%s, rolls again %s',
                       c.id, c.amount, p.id, r.carried, r.n, c.classification, p.classification,
                       c.subscription_id, p.subscription_id, c.application, p.application,
                       c.priority, p.priority, c.effective_at, p.expires_at,
                       c.rollover IS NOT NULL) AS message
         FROM customer_credit_grant c
         JOIN customer_credit_grant p ON p.id = c.source_id
         LEFT JOIN rolled r ON r.grant_id = p.id
         LEFT JOIN products cp ON cp.grant_id = c.id
         LEFT JOIN products pp ON pp.grant_id = p.id
         WHERE c.tenant_id = $1 AND c.source_kind = 'ROLLOVER'
           AND (r.n IS DISTINCT FROM 1 OR r.carried <> c.amount
                OR c.classification <> p.classification
                OR c.subscription_id IS DISTINCT FROM p.subscription_id
                OR c.application <> p.application OR c.priority <> p.priority - 1
                OR c.effective_at IS DISTINCT FROM p.expires_at OR c.rollover IS NOT NULL
                OR cp.ids IS DISTINCT FROM pp.ids
                OR (p.ends_at IS NOT NULL AND p.ends_at <= p.expires_at))",
    ),
    (
        "I13 a grant rolls over once",
        "SELECT format('grant %s rolled over %s times', grant_id, count(*)) AS message
         FROM customer_credit_entry WHERE tenant_id = $1 AND kind = 'ROLLOVER'
         GROUP BY grant_id HAVING count(*) > 1",
    ),
    (
        "D2 one live subscription per usage filter value",
        "SELECT format('subscriptions %s and %s of customer %s both claim %s = %s', a.subscription_id,
                       b.subscription_id, sa.customer_id, a.property, a.value) AS message
         FROM subscription_usage_filter a
         JOIN subscription_usage_filter b
           ON b.tenant_id = a.tenant_id AND b.subscription_id > a.subscription_id
          AND b.property = a.property AND b.value = a.value
          AND a.effective_from < coalesce(b.effective_until, 'infinity')
          AND b.effective_from < coalesce(a.effective_until, 'infinity')
         JOIN subscription sa ON sa.id = a.subscription_id
         JOIN subscription sb ON sb.id = b.subscription_id AND sb.customer_id = sa.customer_id
         WHERE a.tenant_id = $1
           AND sa.status IN ('PENDING_ACTIVATION', 'PENDING_CHARGE', 'TRIAL_ACTIVE', 'ACTIVE')
           AND sb.status IN ('PENDING_ACTIVATION', 'PENDING_CHARGE', 'TRIAL_ACTIVE', 'ACTIVE')",
    ),
];

/// Every invariant violated by `tenant_id`, as "invariant: detail" lines. Reads one snapshot.
pub async fn credit_invariant_violations(conn: &mut PgConn, tenant_id: TenantId) -> Vec<String> {
    conn.batch_execute("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .await
        .expect("open the invariant snapshot");
    let violations = violations_in_snapshot(conn, tenant_id.as_uuid()).await;
    conn.batch_execute("COMMIT")
        .await
        .expect("close the invariant snapshot");
    violations
}

async fn violations_in_snapshot(conn: &mut PgConn, tenant: Uuid) -> Vec<String> {
    let ids = |sql: &'static str| diesel::sql_query(sql).bind::<diesel::sql_types::Uuid, _>(tenant);
    let invoices: Vec<Uuid> = ids("SELECT id FROM invoice WHERE tenant_id = $1")
        .load::<IdRow>(conn)
        .await
        .expect("invoices")
        .into_iter()
        .map(|row| row.id)
        .collect();
    let excess: Vec<Uuid> = invoices
        .iter()
        .map(|id| Uuid::new_v5(id, b"usage-settlement-excess"))
        .collect();
    let settlements: Vec<Uuid> = ids("SELECT id FROM credit_usage_settlement WHERE tenant_id = $1")
        .load::<IdRow>(conn)
        .await
        .expect("settlements")
        .into_iter()
        .map(|row| row.id)
        .collect();
    let give_back: Vec<Uuid> = settlements
        .iter()
        .map(|id| diesel_models::customer_credit::usage_settlement_give_back_source(*id))
        .collect();

    let populated = populated_tables(conn, tenant).await;
    let mut violations = Vec::new();
    for (name, sql) in CHECKS {
        if !may_report(sql, &populated) {
            continue;
        }
        let rows: Vec<Violation> = diesel::sql_query(*sql)
            .bind::<diesel::sql_types::Uuid, _>(tenant)
            .bind::<Array<diesel::sql_types::Uuid>, _>(&invoices)
            .bind::<Array<diesel::sql_types::Uuid>, _>(&excess)
            .bind::<Array<diesel::sql_types::Uuid>, _>(&settlements)
            .bind::<Array<diesel::sql_types::Uuid>, _>(&give_back)
            .load(conn)
            .await
            .unwrap_or_else(|error| panic!("invariant query `{name}` failed: {error:?}"));
        let count = rows.len();
        violations.extend(
            rows.into_iter()
                .take(SHOWN)
                .map(|row| format!("{name}: {}", row.message)),
        );
        if count > SHOWN {
            violations.push(format!("{name}: … {} more", count - SHOWN));
        }
    }
    violations
}

/// Every check reports rows of at least one of these; the tenant's empty ones rule checks out.
const PROBED: [&str; 9] = [
    "invoice",
    "credit_note",
    "customer_credit_account",
    "customer_credit_grant",
    "customer_credit_grant_product",
    "customer_credit_entry",
    "credit_usage_settlement",
    "customer_balance_tx",
    "subscription_usage_filter",
];

#[derive(diesel::QueryableByName)]
struct Populated {
    #[diesel(sql_type = diesel::sql_types::Array<diesel::sql_types::Text>)]
    tables: Vec<String>,
}

async fn populated_tables(conn: &mut PgConn, tenant: Uuid) -> Vec<String> {
    let probes = PROBED
        .iter()
        .map(|t| {
            format!("CASE WHEN EXISTS (SELECT 1 FROM {t} WHERE tenant_id = $1) THEN '{t}' END")
        })
        .collect::<Vec<_>>()
        .join(", ");
    diesel::sql_query(format!(
        "SELECT array_remove(ARRAY[{probes}], NULL) AS tables"
    ))
    .bind::<diesel::sql_types::Uuid, _>(tenant)
    .get_result::<Populated>(conn)
    .await
    .expect("populated tables")
    .tables
}

/// False when every probed table the check reads is empty for the tenant. A check reading
/// none of them always runs.
fn may_report(sql: &str, populated: &[String]) -> bool {
    let words: Vec<&str> = sql
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
        .collect();
    let reads: Vec<&str> = words
        .windows(2)
        .filter(|w| {
            (w[0].eq_ignore_ascii_case("FROM") || w[0].eq_ignore_ascii_case("JOIN"))
                && PROBED.contains(&w[1])
        })
        .map(|w| w[1])
        .collect();
    reads.is_empty() || reads.iter().any(|t| populated.iter().any(|p| p == t))
}

/// Fails the test when `tenant_id` breaks a credit invariant.
pub async fn assert_credit_invariants(store: &Store, tenant_id: TenantId) {
    let mut conn = store.get_conn().await.expect("billing connection");
    let violations = credit_invariant_violations(&mut conn, tenant_id).await;
    assert!(
        violations.is_empty(),
        "credit invariants violated for tenant {tenant_id}:\n{}",
        violations.join("\n")
    );
}

/// The violations of every tenant of the database at `url`, read from a thread of its own: called
/// while the test's runtime is being torn down.
pub fn database_violations(url: &str) -> Vec<String> {
    let url = url.to_owned();
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("invariant runtime")
            .block_on(async move {
                let pool = meteroid_store::store::diesel_make_pg_pool(
                    meteroid_store::store::PgConfig::new(url),
                )
                .expect("invariant pool");
                let mut conn = pool.get().await.expect("invariant connection");
                let tenants: Vec<Uuid> = diesel::sql_query("SELECT id FROM tenant ORDER BY id")
                    .load::<IdRow>(&mut conn)
                    .await
                    .expect("tenants")
                    .into_iter()
                    .map(|row| row.id)
                    .collect();
                let mut violations = Vec::new();
                for tenant in tenants {
                    for violation in
                        credit_invariant_violations(&mut conn, TenantId::from(tenant)).await
                    {
                        violations.push(format!("tenant {tenant}: {violation}"));
                    }
                }
                violations
            })
    })
    .join()
    .expect("the invariant check ran")
}
