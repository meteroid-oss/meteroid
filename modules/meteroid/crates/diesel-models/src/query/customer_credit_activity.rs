//! Read-only views of a customer's credit for the dashboard: how each grant was consumed, what each
//! account added and used over a window, and the statement labelled by where each grant came from.
use crate::customer_credit::CustomerCreditStatementRow;
use crate::errors::IntoDbResult;
use crate::query::customer_credit::STATEMENT_MOVEMENTS;
use crate::{DbResult, PgConn};
use chrono::{DateTime, Utc};
use common_domain::ids::{BaseId, CreditGrantId, CustomerId, SubscriptionId, TenantId, WalletId};
use diesel::QueryableByName;
use diesel::sql_types::{Array, BigInt, Bool, Nullable, Text, Timestamptz, Uuid as SqlUuid};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;
use uuid::Uuid;

const AUTOMATIC_TOP_UP_NAMESPACE: &str = "AUTOMATIC_TOP_UP";

/// Whether a grant whose subscription is `{column}` is credit the subscription `${param}` can spend:
/// its own or the customer-wide credit; every grant when `${param}` is NULL.
fn spendable_by(column: &str, param: u8) -> String {
    format!("(${param}::uuid IS NULL OR {column} IS NULL OR {column} = ${param})")
}

fn subscription_uuid(subscription_id: Option<SubscriptionId>) -> Option<Uuid> {
    subscription_id.map(|id| id.as_uuid())
}

/// The name of the invoice line `gi` that funded grant `g`, when it was funded by one.
// Bonus credit a purchase sold below face is named by its reason, not by the line that sold it.
const GRANT_LINE_NAME: &str = "(CASE WHEN g.source_kind = 'CREDIT_PURCHASE'
    AND g.classification = 'PROMOTIONAL' THEN NULL
    ELSE jsonb_path_query_first(gi.line_items, '$[*] ? (@.local_id == $id).name',
    jsonb_build_object('id', g.line_local_id)) #>> '{}' END)";

/// Per account and credit note, the credit a note took off a grant and funded back as balance
/// (a pack's fee credited back on a late payment, a plan change or a cancellation): one value moved,
/// counted neither as added again nor as removed. Reads the movements of a CTE `w`.
const MOVED_TO_BALANCE: &str = "moved AS (
    SELECT account_id, credit_note_id, least(reversed, funded) AS moved
    FROM (SELECT w.account_id,
                 CASE WHEN w.kind = 'FUND' THEN w.grant_credit_note_id ELSE w.credit_note_id END
                     AS credit_note_id,
                 -coalesce(sum(w.amount) FILTER (WHERE w.kind = 'REVERSAL'), 0) AS reversed,
                 coalesce(sum(w.amount) FILTER (WHERE w.kind = 'FUND'), 0) AS funded
          FROM w
          WHERE (w.kind = 'REVERSAL' AND w.credit_note_id IS NOT NULL)
             OR (w.kind = 'FUND' AND w.source_kind = 'CREDIT_NOTE'
                 AND w.grant_credit_note_id IS NOT NULL)
          GROUP BY 1, 2) n
    WHERE reversed > 0 AND funded > 0)";

fn statement_binds(
    account_ids: &[WalletId],
    rolling: &[(CreditGrantId, i64)],
) -> (Vec<Uuid>, Vec<Uuid>, Vec<i64>) {
    let (rolling_ids, carried) = rolling
        .iter()
        .map(|(grant_id, carried)| (grant_id.as_uuid(), *carried))
        .unzip();
    (
        account_ids.iter().map(|id| id.as_uuid()).collect(),
        rolling_ids,
        carried,
    )
}

#[derive(Debug, Clone, QueryableByName)]
pub struct CreditGrantActivityRow {
    #[diesel(sql_type = SqlUuid)]
    pub grant_id: CreditGrantId,
    #[diesel(sql_type = SqlUuid)]
    pub account_id: WalletId,
    /// Applied to invoices, net of what was given back.
    #[diesel(sql_type = BigInt)]
    pub invoiced: i64,
    /// Paid for usage when it expired, before any invoice took the usage over.
    #[diesel(sql_type = BigInt)]
    pub settled_at_expiry: i64,
    #[diesel(sql_type = BigInt)]
    pub removed: i64,
    #[diesel(sql_type = BigInt)]
    pub rolled_over: i64,
    #[diesel(sql_type = Nullable<Text>)]
    pub invoice_number: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    pub line_name: Option<String>,
    #[diesel(sql_type = Bool)]
    pub automatic_top_up: bool,
}

#[derive(Debug, Clone, QueryableByName)]
pub struct CreditAccountActivityRow {
    #[diesel(sql_type = SqlUuid)]
    pub account_id: WalletId,
    /// Funded, rolled-over credit excepted.
    #[diesel(sql_type = BigInt)]
    pub added: i64,
    #[diesel(sql_type = BigInt)]
    pub invoiced: i64,
    #[diesel(sql_type = BigInt)]
    pub settled_at_expiry: i64,
    #[diesel(sql_type = BigInt)]
    pub removed: i64,
    /// Lapsed unspent, rolled-over credit excepted.
    #[diesel(sql_type = BigInt)]
    pub expired: i64,
}

#[derive(Debug, Clone, QueryableByName)]
pub struct CreditAddedGroupRow {
    #[diesel(sql_type = SqlUuid)]
    pub account_id: WalletId,
    #[diesel(sql_type = Text)]
    pub source_kind: String,
    #[diesel(sql_type = Text)]
    pub classification: String,
    #[diesel(sql_type = Bool)]
    pub added_by_member: bool,
    #[diesel(sql_type = Text)]
    pub name: String,
    #[diesel(sql_type = BigInt)]
    pub grants: i64,
    #[diesel(sql_type = BigInt)]
    pub amount: i64,
}

/// A statement line with what its grant came from. `sequence` and `balance_after` run per account.
#[derive(Debug, Clone, QueryableByName)]
pub struct CustomerCreditLabelledStatementRow {
    #[diesel(embed)]
    pub statement: CustomerCreditStatementRow,
    #[diesel(sql_type = Text)]
    pub grant_source_kind: String,
    #[diesel(sql_type = Text)]
    pub grant_classification: String,
    #[diesel(sql_type = Nullable<SqlUuid>)]
    pub grant_created_by: Option<Uuid>,
    /// The invoice the line moved credit for.
    #[diesel(sql_type = Nullable<Text>)]
    pub invoice_number: Option<String>,
    /// The invoice that funded the grant.
    #[diesel(sql_type = Nullable<Text>)]
    pub grant_invoice_number: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    pub grant_line_name: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    pub credit_note_number: Option<String>,
    #[diesel(sql_type = Bool)]
    pub automatic_top_up: bool,
}

impl CreditGrantActivityRow {
    /// Every grant of the customer, or only `grant_ids`.
    pub async fn list_for_customer(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        grant_ids: Option<&[CreditGrantId]>,
    ) -> DbResult<Vec<Self>> {
        diesel::sql_query(format!(
            "WITH movements AS (
                 SELECT e.grant_id,
                        (-coalesce(sum(e.amount) FILTER (WHERE e.kind IN ('DRAW', 'DRAW_REVERSAL')
                            AND coalesce(e.invoice_id, st.invoice_id) IS NOT NULL), 0))::bigint
                            AS invoiced,
                        (-coalesce(sum(e.amount) FILTER (WHERE e.kind IN ('DRAW', 'DRAW_REVERSAL')
                            AND coalesce(e.invoice_id, st.invoice_id) IS NULL), 0))::bigint
                            AS settled_at_expiry,
                        (-coalesce(sum(e.amount) FILTER (WHERE e.kind IN ('REVERSAL', 'REINSTATEMENT')), 0))::bigint
                            AS removed,
                        (-coalesce(sum(e.amount) FILTER (WHERE e.kind = 'ROLLOVER'), 0))::bigint
                            AS rolled_over
                 FROM customer_credit_entry e
                 LEFT JOIN credit_usage_settlement st
                        ON e.source_kind = 'USAGE_SETTLEMENT' AND st.id = e.source_id
                       AND st.tenant_id = e.tenant_id
                 WHERE e.tenant_id = $1 AND e.customer_id = $2
                   AND ($4::uuid[] IS NULL OR e.grant_id = ANY($4))
                 GROUP BY e.grant_id),
             top_ups AS (
                 SELECT DISTINCT o.invoice_id
                 FROM customer_credit_operation o
                 WHERE o.tenant_id = $1 AND o.customer_id = $2 AND o.namespace = $3
                   AND o.invoice_id IS NOT NULL)
             SELECT g.id AS grant_id, g.account_id,
                    coalesce(m.invoiced, 0)::bigint AS invoiced,
                    coalesce(m.settled_at_expiry, 0)::bigint AS settled_at_expiry,
                    coalesce(m.removed, 0)::bigint AS removed,
                    coalesce(m.rolled_over, 0)::bigint AS rolled_over,
                    gi.invoice_number,
                    {GRANT_LINE_NAME} AS line_name,
                    (t.invoice_id IS NOT NULL) AS automatic_top_up
             FROM customer_credit_grant g
             LEFT JOIN movements m ON m.grant_id = g.id
             LEFT JOIN invoice gi ON gi.id = g.invoice_id AND gi.tenant_id = g.tenant_id
             LEFT JOIN top_ups t ON t.invoice_id = g.invoice_id
             WHERE g.tenant_id = $1 AND g.customer_id = $2
               AND ($4::uuid[] IS NULL OR g.id = ANY($4))"
        ))
        .bind::<SqlUuid, _>(tenant_id.as_uuid())
        .bind::<SqlUuid, _>(customer_id.as_uuid())
        .bind::<Text, _>(AUTOMATIC_TOP_UP_NAMESPACE)
        .bind::<Nullable<Array<SqlUuid>>, _>(
            grant_ids.map(|ids| ids.iter().map(|id| id.as_uuid()).collect::<Vec<_>>()),
        )
        .load(conn)
        .await
        .attach("Error while reading how a customer's credit grants were consumed")
        .into_db_result()
    }
}

impl CreditAccountActivityRow {
    /// Statement movements of `account_ids` that occurred at or after `since` (all when None), with
    /// the same reading of `rolling` and `now` as `CustomerCreditStatementRow::list_for_accounts`.
    /// With `subscription_id`, only the movements of credit it can spend (see `spendable_by`).
    pub async fn list_for_accounts(
        conn: &mut PgConn,
        tenant_id: TenantId,
        account_ids: &[WalletId],
        rolling: &[(CreditGrantId, i64)],
        now: DateTime<Utc>,
        since: Option<DateTime<Utc>>,
        subscription_id: Option<SubscriptionId>,
    ) -> DbResult<Vec<Self>> {
        let (accounts, rolling_ids, carried) = statement_binds(account_ids, rolling);
        let spendable = spendable_by("m.grant_subscription_id", 7);
        diesel::sql_query(format!(
            "WITH w AS (
                 SELECT m.*, g.source_kind, g.credit_note_id AS grant_credit_note_id
                 FROM ({STATEMENT_MOVEMENTS}) m
                 JOIN customer_credit_grant g ON g.id = m.grant_id AND g.tenant_id = $1
                 WHERE ($6::timestamptz IS NULL OR m.occurred_at >= $6) AND {spendable}),
             {MOVED_TO_BALANCE},
             totals AS (
                 SELECT w.account_id,
                        coalesce(sum(w.amount) FILTER (WHERE w.kind = 'FUND' AND w.source_kind <> 'ROLLOVER'), 0)
                            AS added,
                        (-coalesce(sum(w.amount) FILTER (WHERE w.kind IN ('DRAW', 'DRAW_REVERSAL')
                            AND w.invoice_id IS NOT NULL), 0)) AS invoiced,
                        (-coalesce(sum(w.amount) FILTER (WHERE w.kind IN ('DRAW', 'DRAW_REVERSAL')
                            AND w.invoice_id IS NULL), 0)) AS settled_at_expiry,
                        (-coalesce(sum(w.amount) FILTER (WHERE w.kind IN ('REVERSAL', 'REINSTATEMENT')), 0))
                            AS removed,
                        (-coalesce(sum(w.amount) FILTER (WHERE w.kind = 'EXPIRY'), 0)) AS expired
                 FROM w
                 GROUP BY w.account_id)
             SELECT t.account_id,
                    (t.added - coalesce(mv.moved, 0))::bigint AS added,
                    t.invoiced::bigint AS invoiced,
                    t.settled_at_expiry::bigint AS settled_at_expiry,
                    (t.removed - coalesce(mv.moved, 0))::bigint AS removed,
                    t.expired::bigint AS expired
             FROM totals t
             LEFT JOIN (SELECT account_id, sum(moved) AS moved FROM moved GROUP BY account_id) mv
                 ON mv.account_id = t.account_id"
        ))
        .bind::<SqlUuid, _>(tenant_id.as_uuid())
        .bind::<Array<SqlUuid>, _>(accounts)
        .bind::<Array<SqlUuid>, _>(rolling_ids)
        .bind::<Array<BigInt>, _>(carried)
        .bind::<Timestamptz, _>(now)
        .bind::<Nullable<Timestamptz>, _>(since)
        .bind::<Nullable<SqlUuid>, _>(subscription_uuid(subscription_id))
        .load(conn)
        .await
        .attach("Error while totalling a customer's credit activity")
        .into_db_result()
    }
}

impl CreditAddedGroupRow {
    /// Funding that occurred at or after `since` (all of it when None), grouped by source and name.
    /// With `subscription_id`, only the credit it can spend (see `spendable_by`).
    pub async fn list_for_customer(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        since: Option<DateTime<Utc>>,
        subscription_id: Option<SubscriptionId>,
    ) -> DbResult<Vec<Self>> {
        let spendable = spendable_by("g.subscription_id", 4);
        diesel::sql_query(format!(
            "WITH w AS (
                 SELECT e.grant_id, e.kind, e.amount, e.credit_note_id, g.account_id,
                        g.source_kind, g.credit_note_id AS grant_credit_note_id
                 FROM customer_credit_entry e
                 JOIN customer_credit_grant g ON g.id = e.grant_id AND g.tenant_id = e.tenant_id
                 WHERE e.tenant_id = $1 AND e.customer_id = $2
                   AND ($3::timestamptz IS NULL OR e.occurred_at >= $3) AND {spendable}),
             {MOVED_TO_BALANCE},
             funded AS (
                 SELECT w.grant_id,
                        sum(w.amount) - coalesce(max(mv.moved), 0) AS amount
                 FROM w
                 LEFT JOIN moved mv ON mv.account_id = w.account_id
                     AND mv.credit_note_id = w.grant_credit_note_id AND w.source_kind = 'CREDIT_NOTE'
                 WHERE w.kind = 'FUND' AND w.source_kind <> 'ROLLOVER'
                 GROUP BY w.grant_id)
             SELECT g.account_id, g.source_kind, g.classification,
                    (g.created_by IS NOT NULL) AS added_by_member,
                    coalesce({GRANT_LINE_NAME}, g.reason) AS name,
                    count(DISTINCT g.id)::bigint AS grants,
                    sum(f.amount)::bigint AS amount
             FROM funded f
             JOIN customer_credit_grant g ON g.id = f.grant_id AND g.tenant_id = $1
             LEFT JOIN invoice gi ON gi.id = g.invoice_id AND gi.tenant_id = g.tenant_id
             WHERE f.amount <> 0
             GROUP BY 1, 2, 3, 4, 5
             ORDER BY amount DESC, name"
        ))
        .bind::<SqlUuid, _>(tenant_id.as_uuid())
        .bind::<SqlUuid, _>(customer_id.as_uuid())
        .bind::<Nullable<Timestamptz>, _>(since)
        .bind::<Nullable<SqlUuid>, _>(subscription_uuid(subscription_id))
        .load(conn)
        .await
        .attach("Error while grouping a customer's credit funding")
        .into_db_result()
    }
}

impl CustomerCreditLabelledStatementRow {
    /// The statement of `account_ids` as of `now`, newest first; `rolling` holds, per expired grant
    /// the sweep has not rolled over yet, what it will carry. A rollover is one row, the funding of
    /// the grant it carries into: its carry-out and carry-in cancel, so it moves no balance. With
    /// `subscription_id`, the statement of the credit it can spend (see `spendable_by`): the
    /// sequence and running balance count those movements only.
    #[allow(clippy::too_many_arguments)]
    pub async fn list_for_accounts(
        conn: &mut PgConn,
        tenant_id: TenantId,
        account_ids: &[WalletId],
        rolling: &[(CreditGrantId, i64)],
        now: DateTime<Utc>,
        subscription_id: Option<SubscriptionId>,
        limit: i64,
        offset: i64,
    ) -> DbResult<Vec<Self>> {
        let (accounts, rolling_ids, carried) = statement_binds(account_ids, rolling);
        let spendable = spendable_by("m.grant_subscription_id", 9);
        diesel::sql_query(format!(
            "WITH m AS ({STATEMENT_MOVEMENTS}),
             d AS (
                 SELECT m.*,
                        CASE WHEN m.kind = 'FUND' AND rg.source_kind = 'ROLLOVER' THEN 0
                             ELSE m.amount END AS net
                 FROM m
                 JOIN customer_credit_grant rg ON rg.id = m.grant_id AND rg.tenant_id = $1
                 WHERE m.kind <> 'ROLLOVER' AND {spendable}),
             s AS (
                 SELECT d.*,
                        row_number() OVER w AS sequence,
                        (sum(d.net) OVER w)::bigint AS balance_after
                 FROM d
                 WINDOW w AS (PARTITION BY d.account_id
                              ORDER BY d.occurred_at, d.unrecorded, d.recorded_at, d.id
                              ROWS UNBOUNDED PRECEDING)),
             top_ups AS (
                 SELECT DISTINCT o.invoice_id
                 FROM customer_credit_operation o
                 WHERE o.tenant_id = $1 AND o.account_id = ANY($2) AND o.namespace = $8
                   AND o.invoice_id IS NOT NULL)
             SELECT s.*,
                    g.source_kind AS grant_source_kind,
                    g.classification AS grant_classification,
                    g.created_by AS grant_created_by,
                    ei.invoice_number,
                    gi.invoice_number AS grant_invoice_number,
                    {GRANT_LINE_NAME} AS grant_line_name,
                    cn.credit_note_number,
                    (t.invoice_id IS NOT NULL) AS automatic_top_up
             FROM s
             JOIN customer_credit_grant g ON g.id = s.grant_id AND g.tenant_id = $1
             LEFT JOIN invoice ei ON ei.id = s.invoice_id AND ei.tenant_id = $1
             LEFT JOIN invoice gi ON gi.id = g.invoice_id AND gi.tenant_id = $1
             LEFT JOIN credit_note cn ON cn.id = s.credit_note_id AND cn.tenant_id = $1
             LEFT JOIN top_ups t ON t.invoice_id = g.invoice_id
             ORDER BY s.occurred_at DESC, s.unrecorded DESC, s.recorded_at DESC, s.id DESC
             LIMIT $6 OFFSET $7"
        ))
        .bind::<SqlUuid, _>(tenant_id.as_uuid())
        .bind::<Array<SqlUuid>, _>(accounts)
        .bind::<Array<SqlUuid>, _>(rolling_ids)
        .bind::<Array<BigInt>, _>(carried)
        .bind::<Timestamptz, _>(now)
        .bind::<BigInt, _>(limit)
        .bind::<BigInt, _>(offset)
        .bind::<Text, _>(AUTOMATIC_TOP_UP_NAMESPACE)
        .bind::<Nullable<SqlUuid>, _>(subscription_uuid(subscription_id))
        .load(conn)
        .await
        .attach("Error while reading a labelled customer credit statement")
        .into_db_result()
    }

    pub async fn count_for_accounts(
        conn: &mut PgConn,
        tenant_id: TenantId,
        account_ids: &[WalletId],
        rolling: &[(CreditGrantId, i64)],
        now: DateTime<Utc>,
        subscription_id: Option<SubscriptionId>,
    ) -> DbResult<i64> {
        #[derive(QueryableByName)]
        struct Count {
            #[diesel(sql_type = BigInt)]
            total: i64,
        }
        let (accounts, rolling_ids, carried) = statement_binds(account_ids, rolling);
        let spendable = spendable_by("m.grant_subscription_id", 6);
        diesel::sql_query(format!(
            "SELECT count(*) AS total FROM ({STATEMENT_MOVEMENTS}) m
             WHERE m.kind <> 'ROLLOVER' AND {spendable}"
        ))
        .bind::<SqlUuid, _>(tenant_id.as_uuid())
        .bind::<Array<SqlUuid>, _>(accounts)
        .bind::<Array<SqlUuid>, _>(rolling_ids)
        .bind::<Array<BigInt>, _>(carried)
        .bind::<Timestamptz, _>(now)
        .bind::<Nullable<SqlUuid>, _>(subscription_uuid(subscription_id))
        .get_result::<Count>(conn)
        .await
        .map(|row| row.total)
        .attach("Error while counting a labelled customer credit statement")
        .into_db_result()
    }
}
