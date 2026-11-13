//! Invoice-time credit usage: the settlements credit pays ahead of an invoice, the reads core
//! shares with the live engine, and the recompute marks every credit write leaves.
use crate::customer_credit::{
    CreditRecomputeCandidate, CreditRecomputeMarkRow, CreditRecomputeMarkRowNew,
    CreditUsageSettlementRow, CreditUsageSettlementRowNew, CustomerCreditAccountRow,
    CustomerCreditGrantRow, ScopedGrantRow,
};
use crate::errors::IntoDbResult;
use crate::lock::CustomerLock;
use crate::{DbResult, PgConn};
use chrono::{DateTime, NaiveDate, Utc};
use common_domain::ids::{
    BaseId, CustomerId, InvoiceId, ProductId, SubscriptionId, TenantId, UsageSettlementId, WalletId,
};
use diesel::upsert::DecoratableTarget;
use diesel::{
    ExpressionMethods, JoinOnDsl, OptionalExtension, QueryDsl, SelectableHelper, sql_types,
};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;
use uuid::Uuid;

impl CreditUsageSettlementRow {
    /// Records it after the account's settlements of the same period and bound (usage that reached
    /// the metering store after them). None when one of those was taken over by an invoice (that
    /// one marks the period settled, even voided), or a concurrent writer took the place.
    pub async fn insert_new(
        conn: &mut PgConn,
        row: &CreditUsageSettlementRowNew,
    ) -> DbResult<Option<Self>> {
        use crate::schema::credit_usage_settlement::dsl as s;
        let give_back = crate::customer_credit::usage_settlement_give_back_source(*row.id);
        let earlier: Vec<(i32, Option<InvoiceId>)> = s::credit_usage_settlement
            .filter(s::account_id.eq(row.account_id))
            .filter(s::subscription_id.eq(row.subscription_id))
            .filter(s::period_start.eq(row.period_start))
            .filter(s::event_time_bound.eq(row.event_time_bound))
            .filter(s::returned_at.is_null())
            .select((s::ordinal, s::invoice_id))
            .load(conn)
            .await
            .attach("Error while reading the settlements of a bound")
            .into_db_result()?;
        if earlier.iter().any(|(_, invoice)| invoice.is_some()) {
            return Ok(None);
        }
        let last = earlier.iter().map(|(ordinal, _)| *ordinal).max();
        let ordinal = last.map_or(0, |last| last.saturating_add(1));
        diesel::insert_into(s::credit_usage_settlement)
            .values((
                row,
                s::give_back_source_id.eq(give_back),
                s::ordinal.eq(ordinal),
                s::created_at.eq(common_domain::clock::now()),
            ))
            .on_conflict((
                s::account_id,
                s::subscription_id,
                s::period_start,
                s::event_time_bound,
                s::ordinal,
            ))
            .filter_target(s::returned_at.is_null())
            .do_nothing()
            .returning(Self::as_returning())
            .get_result(conn)
            .await
            .optional()
            .attach("Error while recording a usage settlement")
            .into_db_result()
    }

    /// A period's settlements paid by wallets of one denomination, except those given back while
    /// open. One taken over by a since-voided invoice stays listed on purpose: it marks the period
    /// as already settled, so the expired credit its void gave back never settles it again.
    pub async fn list_for_period(
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        period_start: NaiveDate,
        currency: &str,
    ) -> DbResult<Vec<Self>> {
        use crate::schema::credit_usage_settlement::dsl as s;
        use crate::schema::customer_credit_account::dsl as a;
        s::credit_usage_settlement
            .inner_join(a::customer_credit_account.on(a::id.eq(s::account_id)))
            .filter(s::tenant_id.eq(tenant_id))
            .filter(s::subscription_id.eq(subscription_id))
            .filter(s::period_start.eq(period_start))
            .filter(s::returned_at.is_null())
            .filter(a::currency.eq(currency))
            .order((s::event_time_bound.asc(), s::id.asc()))
            .select(Self::as_select())
            .load(conn)
            .await
            .attach("Error while reading a period's usage settlements")
            .into_db_result()
    }

    /// Hands the settlements of a period that `account_ids` paid and no invoice has taken yet to
    /// `invoice_id`: an invoice takes each denomination's settlements under that one's grant locks.
    pub async fn attach_open_to_invoice(
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        period_start: NaiveDate,
        account_ids: &[WalletId],
        invoice_id: InvoiceId,
    ) -> DbResult<Vec<Self>> {
        use crate::schema::credit_usage_settlement::dsl as s;
        if account_ids.is_empty() {
            return Ok(Vec::new());
        }
        diesel::update(
            s::credit_usage_settlement
                .filter(s::tenant_id.eq(tenant_id))
                .filter(s::subscription_id.eq(subscription_id))
                .filter(s::period_start.eq(period_start))
                .filter(s::account_id.eq_any(account_ids))
                .filter(s::invoice_id.is_null())
                .filter(s::returned_at.is_null()),
        )
        .set(s::invoice_id.eq(invoice_id))
        .returning(Self::as_returning())
        .get_results(conn)
        .await
        .attach("Error while attaching usage settlements to an invoice")
        .into_db_result()
    }

    /// Marks an open settlement as given back; none when an invoice took it or it was already returned.
    pub async fn mark_returned(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: UsageSettlementId,
    ) -> DbResult<Option<Self>> {
        use crate::schema::credit_usage_settlement::dsl as s;
        diesel::update(
            s::credit_usage_settlement
                .filter(s::tenant_id.eq(tenant_id))
                .filter(s::id.eq(id))
                .filter(s::invoice_id.is_null())
                .filter(s::returned_at.is_null()),
        )
        .set(s::returned_at.eq(common_domain::clock::now().naive_utc()))
        .returning(Self::as_returning())
        .get_result(conn)
        .await
        .optional()
        .attach("Error while returning a usage settlement")
        .into_db_result()
    }

    /// Customers with settlements open since before `created_before`, in `(tenant, customer)`
    /// order after `after`.
    pub async fn list_customers_with_open_before(
        conn: &mut PgConn,
        created_before: DateTime<Utc>,
        after: Option<(TenantId, CustomerId)>,
        limit: i64,
    ) -> DbResult<Vec<(TenantId, CustomerId)>> {
        let (after_tenant, after_customer) = after
            .map(|(t, c)| (t.as_uuid(), c.as_uuid()))
            .unwrap_or((Uuid::nil(), Uuid::nil()));
        let rows = diesel::sql_query(
            "SELECT tenant_id, customer_id
             FROM credit_usage_settlement
             WHERE invoice_id IS NULL AND returned_at IS NULL AND created_at < $1
               AND (tenant_id, customer_id) > ($2, $3)
             GROUP BY tenant_id, customer_id
             ORDER BY tenant_id, customer_id
             LIMIT $4",
        )
        .bind::<sql_types::Timestamptz, _>(created_before)
        .bind::<sql_types::Uuid, _>(after_tenant)
        .bind::<sql_types::Uuid, _>(after_customer)
        .bind::<sql_types::BigInt, _>(limit)
        .get_results::<CreditRecomputeCandidate>(conn)
        .await
        .attach("Error while listing customers with open usage settlements")
        .into_db_result()?;
        Ok(rows
            .into_iter()
            .map(|r| (r.tenant_id, r.customer_id))
            .collect())
    }

    pub async fn list_by_invoice(
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> DbResult<Vec<Self>> {
        use crate::schema::credit_usage_settlement::dsl as s;
        s::credit_usage_settlement
            .filter(s::tenant_id.eq(tenant_id))
            .filter(s::invoice_id.eq(invoice_id))
            .order((s::event_time_bound.asc(), s::id.asc()))
            .select(Self::as_select())
            .load(conn)
            .await
            .attach("Error while reading an invoice's usage settlements")
            .into_db_result()
    }

    pub async fn find_by_id(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: UsageSettlementId,
    ) -> DbResult<Option<Self>> {
        use crate::schema::credit_usage_settlement::dsl as s;
        s::credit_usage_settlement
            .filter(s::tenant_id.eq(tenant_id))
            .filter(s::id.eq(id))
            .select(Self::as_select())
            .first(conn)
            .await
            .optional()
            .attach("Error while reading a usage settlement")
            .into_db_result()
    }
}

impl CreditUsageSettlementRow {
    /// Settlements no invoice has taken over yet, whichever their period.
    pub async fn list_open_for_customers(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_ids: &[CustomerId],
    ) -> DbResult<Vec<Self>> {
        use crate::schema::credit_usage_settlement::dsl as s;
        s::credit_usage_settlement
            .filter(s::tenant_id.eq(tenant_id))
            .filter(s::customer_id.eq_any(customer_ids))
            .filter(s::invoice_id.is_null())
            .filter(s::returned_at.is_null())
            .select(Self::as_select())
            .load(conn)
            .await
            .attach("Error while reading open usage settlements")
            .into_db_result()
    }
}

impl CustomerCreditGrantRow {
    /// Grants holding value, for each customer, with their products and the currency of their
    /// account.
    pub async fn list_available_for_customers(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_ids: &[CustomerId],
    ) -> DbResult<Vec<(ScopedGrantRow, String)>> {
        use diesel::QueryableByName;
        use diesel::sql_types::{Array, Uuid as SqlUuid};
        #[derive(QueryableByName)]
        struct Row {
            #[diesel(embed)]
            scoped: ScopedGrantRow,
            #[diesel(sql_type = diesel::sql_types::Text)]
            currency: String,
        }
        let customers = customer_ids
            .iter()
            .map(|id| id.as_uuid())
            .collect::<Vec<_>>();
        let rows: Vec<Row> = diesel::sql_query(
            "WITH gp AS (
                 SELECT p.grant_id, array_agg(p.product_id ORDER BY p.product_id) AS product_ids
                 FROM customer_credit_grant_product p
                 JOIN customer_credit_grant c ON c.id = p.grant_id
                 WHERE p.tenant_id = $1 AND c.tenant_id = $1 AND c.customer_id = ANY($2)
                   AND c.remaining > 0
                 GROUP BY p.grant_id)
             SELECT g.*, coalesce(gp.product_ids, '{}'::uuid[]) AS product_ids, a.currency
             FROM customer_credit_grant g
             JOIN customer_credit_account a ON a.id = g.account_id
             LEFT JOIN gp ON gp.grant_id = g.id
             WHERE g.tenant_id = $1 AND g.customer_id = ANY($2) AND g.remaining > 0
             ORDER BY g.id",
        )
        .bind::<SqlUuid, _>(tenant_id.as_uuid())
        .bind::<Array<SqlUuid>, _>(customers)
        .load(conn)
        .await
        .attach("Error while reading available customer credit")
        .into_db_result()?;
        Ok(rows
            .into_iter()
            .map(|row| (row.scoped, row.currency))
            .collect())
    }
}

impl CustomerCreditAccountRow {
    /// Supersedes every live projection of the customer, in the transaction of each ledger
    /// movement, and writes the movement's change-feed row. Locks all its wallets, as
    /// `lock_by_id` would each.
    pub async fn bump_ledger_version(conn: &mut PgConn, lock: &CustomerLock) -> DbResult<usize> {
        use crate::schema::customer_credit_account::dsl as a;
        lock.on(conn);
        let bumped = diesel::update(
            a::customer_credit_account
                .filter(a::tenant_id.eq(lock.tenant_id()))
                .filter(a::customer_id.eq(lock.customer_id())),
        )
        .set(a::ledger_version.eq(a::ledger_version + 1))
        .execute(conn)
        .await
        .attach("Error while recording a customer's ledger movement")
        .into_db_result()?;
        crate::query::credit_inputs::record_ledger_change(conn, lock).await?;
        Ok(bumped)
    }

    pub async fn list_for_customers(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_ids: &[CustomerId],
    ) -> DbResult<Vec<Self>> {
        use crate::schema::customer_credit_account::dsl as a;
        a::customer_credit_account
            .filter(a::tenant_id.eq(tenant_id))
            .filter(a::customer_id.eq_any(customer_ids))
            .order((a::customer_id.asc(), a::currency.asc()))
            .select(Self::as_select())
            .load(conn)
            .await
            .attach("Error while listing customer credit accounts")
            .into_db_result()
    }
}

const MARK_BATCH: usize = 5_000;

impl CreditRecomputeMarkRow {
    pub async fn upsert_many(
        conn: &mut PgConn,
        marks: &[(TenantId, CustomerId)],
    ) -> DbResult<usize> {
        use crate::schema::credit_recompute_mark::dsl as m;
        if marks.is_empty() {
            return Ok(0);
        }
        let rows = marks
            .iter()
            .map(|(tenant_id, customer_id)| CreditRecomputeMarkRowNew {
                tenant_id: *tenant_id,
                customer_id: *customer_id,
            })
            .collect::<Vec<_>>();
        diesel::insert_into(m::credit_recompute_mark)
            .values(&rows)
            .on_conflict((m::tenant_id, m::customer_id))
            .do_update()
            .set(m::marked_at.eq(diesel::dsl::sql::<sql_types::Timestamptz>(
                "clock_timestamp()",
            )))
            .execute(conn)
            .await
            .attach("Error while marking customers for a credit recompute")
            .into_db_result()
    }

    pub async fn mark_one(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
    ) -> DbResult<usize> {
        Self::upsert_many(conn, &[(tenant_id, customer_id)]).await
    }

    /// Marks, in one statement, the customers an entitlement of the given owner reaches: those
    /// with an active subscription on the plan, plan version or add-on, the owning subscription
    /// or customer, or, for a feature, those whose subscription carries its product. Quotes reach
    /// no customer with a subscription. At most `MARK_BATCH` are marked, so an edit never turns
    /// into a write as large as the tenant; the sweep recomputes the rest within its cadence.
    pub async fn mark_for_entitlement_owner(
        conn: &mut PgConn,
        tenant_id: TenantId,
        entity_type: &crate::enums::EntitlementEntityTypeEnum,
        entity_id: Uuid,
    ) -> DbResult<usize> {
        use crate::enums::EntitlementEntityTypeEnum as Owner;
        const ACTIVE: &str = "s.status IN ('ACTIVE', 'TRIAL_ACTIVE', 'PENDING_CHARGE')";
        let selection = match entity_type {
            Owner::Customer => {
                return Self::mark_one(conn, tenant_id, CustomerId::from(entity_id)).await;
            }
            Owner::Quote => return Ok(0),
            Owner::Subscription => "FROM subscription s WHERE s.tenant_id = $1 AND s.id = $2".to_owned(),
            Owner::PlanVersion => format!(
                "FROM subscription s WHERE s.tenant_id = $1 AND s.plan_version_id = $2 AND {ACTIVE}"
            ),
            Owner::Plan => format!(
                "FROM subscription s JOIN plan_version pv ON pv.id = s.plan_version_id
                 WHERE s.tenant_id = $1 AND pv.plan_id = $2 AND {ACTIVE}"
            ),
            Owner::AddOn => format!(
                "FROM subscription s
                 JOIN subscription_add_on sa ON sa.subscription_id = s.id AND sa.effective_to IS NULL
                 WHERE s.tenant_id = $1 AND sa.add_on_id = $2 AND {ACTIVE}"
            ),
            Owner::Feature => format!(
                "FROM subscription s
                 JOIN feature f ON f.id = $2 AND f.tenant_id = s.tenant_id
                 LEFT JOIN (
                     SELECT subscription_id, product_id FROM subscription_component
                     UNION
                     SELECT subscription_id, product_id FROM subscription_add_on
                     WHERE effective_to IS NULL
                 ) sp ON sp.subscription_id = s.id AND sp.product_id = f.product_id
                 WHERE s.tenant_id = $1 AND {ACTIVE}
                   AND (f.product_id IS NULL OR sp.product_id IS NOT NULL)"
            ),
        };
        Self::mark_selection(conn, &selection, tenant_id, Some(entity_id)).await
    }

    /// Marks the customers with an active subscription that carries the product, or every one
    /// when there is none: the customers a feature reached through that product.
    pub async fn mark_for_feature_product(
        conn: &mut PgConn,
        tenant_id: TenantId,
        product_id: Option<ProductId>,
    ) -> DbResult<usize> {
        let selection = match product_id {
            Some(_) => "FROM subscription s
                 JOIN (
                     SELECT subscription_id, product_id FROM subscription_component
                     UNION
                     SELECT subscription_id, product_id FROM subscription_add_on
                     WHERE effective_to IS NULL
                 ) sp ON sp.subscription_id = s.id AND sp.product_id = $2
                 WHERE s.tenant_id = $1
                   AND s.status IN ('ACTIVE', 'TRIAL_ACTIVE', 'PENDING_CHARGE')"
                .to_owned(),
            None => "FROM subscription s
                 WHERE s.tenant_id = $1 AND $2::uuid IS NULL
                   AND s.status IN ('ACTIVE', 'TRIAL_ACTIVE', 'PENDING_CHARGE')"
                .to_owned(),
        };
        Self::mark_selection(
            conn,
            &selection,
            tenant_id,
            product_id.map(|id| id.as_uuid()),
        )
        .await
    }

    /// Marks the tenant's live customers, at most `MARK_BATCH`: the leader recomputes those in
    /// scope at once, the sweep the rest.
    pub async fn mark_tenant_customers(conn: &mut PgConn, tenant_id: TenantId) -> DbResult<usize> {
        diesel::sql_query(format!(
            "INSERT INTO credit_recompute_mark (tenant_id, customer_id)
             SELECT c.tenant_id, c.id FROM customer c
             WHERE c.tenant_id = $1 AND c.archived_at IS NULL
             ORDER BY c.id
             LIMIT {MARK_BATCH}
             ON CONFLICT (tenant_id, customer_id) DO UPDATE SET marked_at = clock_timestamp()"
        ))
        .bind::<sql_types::Uuid, _>(tenant_id.as_uuid())
        .execute(conn)
        .await
        .attach("Error while marking a tenant's customers for a credit recompute")
        .into_db_result()
    }

    async fn mark_selection(
        conn: &mut PgConn,
        selection: &str,
        tenant_id: TenantId,
        entity_id: Option<Uuid>,
    ) -> DbResult<usize> {
        let marked = diesel::sql_query(format!(
            "INSERT INTO credit_recompute_mark (tenant_id, customer_id)
             SELECT DISTINCT s.tenant_id, s.customer_id {selection}
             LIMIT {MARK_BATCH}
             ON CONFLICT (tenant_id, customer_id) DO UPDATE SET marked_at = clock_timestamp()"
        ))
        .bind::<sql_types::Uuid, _>(tenant_id.as_uuid())
        .bind::<sql_types::Nullable<sql_types::Uuid>, _>(entity_id)
        .execute(conn)
        .await
        .attach("Error while marking the customers of an entitlement owner")
        .into_db_result()?;
        if marked >= MARK_BATCH {
            log::info!(
                "recompute marks capped at {MARK_BATCH} customers; the sweep covers the rest"
            );
        }
        Ok(marked)
    }
}

/// A customer's draft recurring or final invoice of a subscription: the anchor date and cycle its
/// usage windows are derived from, and its lines as stored.
#[derive(Debug, Clone)]
pub struct DraftRecurringPeriod {
    pub subscription_id: SubscriptionId,
    pub anchor: NaiveDate,
    pub cycle_index: Option<i32>,
    pub tax_inclusive: bool,
    pub line_items: serde_json::Value,
    pub invoice_type: crate::enums::InvoiceType,
}

/// Drafts merged into a consolidated invoice stay Draft, but their periods are billed by it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsolidatedDrafts {
    Include,
    Exclude,
}

pub async fn list_draft_recurring_periods(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
    consolidated: ConsolidatedDrafts,
) -> DbResult<Vec<DraftRecurringPeriod>> {
    use crate::enums::{InvoiceStatusEnum, InvoiceType};
    use crate::schema::invoice::dsl as i;
    type Row = (
        Option<SubscriptionId>,
        NaiveDate,
        Option<i32>,
        bool,
        serde_json::Value,
        InvoiceType,
    );
    let mut query = i::invoice
        .filter(i::tenant_id.eq(tenant_id))
        .filter(i::customer_id.eq(customer_id))
        .filter(i::status.eq(InvoiceStatusEnum::Draft))
        .filter(i::invoice_type.eq_any([InvoiceType::Recurring, InvoiceType::Final]))
        .filter(i::subscription_id.is_not_null())
        .into_boxed();
    if consolidated == ConsolidatedDrafts::Exclude {
        query = query.filter(i::consolidated_into_invoice_id.is_null());
    }
    let rows: Vec<Row> = query
        .order(i::id.asc())
        .select((
            i::subscription_id,
            i::billing_period_start,
            i::cycle_index,
            i::tax_inclusive,
            i::line_items,
            i::invoice_type,
        ))
        .load(conn)
        .await
        .attach("Error while reading draft invoice periods")
        .into_db_result()?;
    Ok(rows
        .into_iter()
        .filter_map(
            |(subscription_id, anchor, cycle_index, tax_inclusive, line_items, invoice_type)| {
                Some(DraftRecurringPeriod {
                    subscription_id: subscription_id?,
                    anchor,
                    cycle_index,
                    tax_inclusive,
                    line_items,
                    invoice_type,
                })
            },
        )
        .collect())
}

/// The anchor date and cycle of a customer's recurring and final invoices dated in
/// `[anchor_from, anchor_before)`, voided ones excluded: every billing period whose usage is (or
/// is about to be) invoiced.
#[derive(Debug, Clone)]
pub struct RecurringInvoiceCycle {
    pub subscription_id: SubscriptionId,
    pub anchor: NaiveDate,
    pub cycle_index: Option<i32>,
    pub invoice_type: crate::enums::InvoiceType,
}

pub async fn list_recurring_invoice_cycles(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
    anchor_from: NaiveDate,
    anchor_before: NaiveDate,
) -> DbResult<Vec<RecurringInvoiceCycle>> {
    use crate::enums::{InvoiceStatusEnum, InvoiceType};
    use crate::schema::invoice::dsl as i;
    let rows: Vec<(Option<SubscriptionId>, NaiveDate, Option<i32>, InvoiceType)> = i::invoice
        .filter(i::tenant_id.eq(tenant_id))
        .filter(i::customer_id.eq(customer_id))
        .filter(i::invoice_type.eq_any([InvoiceType::Recurring, InvoiceType::Final]))
        .filter(i::status.ne(InvoiceStatusEnum::Void))
        .filter(i::subscription_id.is_not_null())
        .filter(i::billing_period_start.ge(anchor_from))
        .filter(i::billing_period_start.lt(anchor_before))
        .order((i::billing_period_start.asc(), i::id.asc()))
        .select((
            i::subscription_id,
            i::billing_period_start,
            i::cycle_index,
            i::invoice_type,
        ))
        .load(conn)
        .await
        .attach("Error while reading recurring invoice cycles")
        .into_db_result()?;
    Ok(rows
        .into_iter()
        .filter_map(|(subscription_id, anchor, cycle_index, invoice_type)| {
            Some(RecurringInvoiceCycle {
                subscription_id: subscription_id?,
                anchor,
                cycle_index,
                invoice_type,
            })
        })
        .collect())
}
