use crate::enums::{InvoiceStatusEnum, ScheduledEventStatus, ScheduledEventTypeEnum};
use crate::errors::IntoDbResult;
use crate::invoices::InvoiceRow;
use crate::{DbResult, PgConn};
use chrono::NaiveDateTime;
use common_domain::ids::{
    CustomerId, InvoiceId, SubscriptionAddOnId, SubscriptionId, SubscriptionPriceComponentId,
    TenantId,
};
use diesel::{
    BoolExpressionMethods, ExpressionMethods, JoinOnDsl, OptionalExtension, QueryDsl,
    SelectableHelper,
};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

impl InvoiceRow {
    /// The customer's first `limit` drafts in the order they finalize: by invoice date, then age.
    /// Consolidated children are left out, their parent being the document that finalizes.
    pub async fn list_customer_drafts(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        limit: i64,
    ) -> DbResult<Vec<InvoiceRow>> {
        use crate::schema::invoice::dsl as i;
        i::invoice
            .filter(i::tenant_id.eq(tenant_id))
            .filter(i::customer_id.eq(customer_id))
            .filter(i::status.eq(InvoiceStatusEnum::Draft))
            .filter(i::consolidated_into_invoice_id.is_null())
            .order((i::invoice_date.asc(), i::created_at.asc(), i::id.asc()))
            .limit(limit)
            .select(InvoiceRow::as_select())
            .load(conn)
            .await
            .attach("Error while listing a customer's draft invoices")
            .into_db_result()
    }

    /// Every draft of the customer finalizing before `invoice_id`, in the order of
    /// `list_customer_drafts`; none when `invoice_id` is not such a draft itself.
    pub async fn list_customer_drafts_before(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        invoice_id: InvoiceId,
    ) -> DbResult<Vec<InvoiceRow>> {
        use crate::schema::invoice::dsl as i;
        let drafts = || {
            i::invoice
                .filter(i::tenant_id.eq(tenant_id))
                .filter(i::customer_id.eq(customer_id))
                .filter(i::status.eq(InvoiceStatusEnum::Draft))
                .filter(i::consolidated_into_invoice_id.is_null())
        };
        let Some((date, created_at)) = drafts()
            .filter(i::id.eq(invoice_id))
            .select((i::invoice_date, i::created_at))
            .first::<(chrono::NaiveDate, NaiveDateTime)>(conn)
            .await
            .optional()
            .attach("Error while reading a draft invoice")
            .into_db_result()?
        else {
            return Ok(Vec::new());
        };
        drafts()
            .filter(
                i::invoice_date.lt(date).or(i::invoice_date.eq(date).and(
                    i::created_at
                        .lt(created_at)
                        .or(i::created_at.eq(created_at).and(i::id.lt(invoice_id))),
                )),
            )
            .order((i::invoice_date.asc(), i::created_at.asc(), i::id.asc()))
            .select(InvoiceRow::as_select())
            .load(conn)
            .await
            .attach("Error while listing a customer's draft invoices")
            .into_db_result()
    }
}

/// The pending cancellations and pauses of the customer's subscriptions: when each stops.
pub async fn list_scheduled_stops(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
) -> DbResult<Vec<(SubscriptionId, NaiveDateTime)>> {
    use crate::schema::scheduled_event::dsl as e;
    use crate::schema::subscription::dsl as s;
    e::scheduled_event
        .inner_join(s::subscription.on(s::id.eq(e::subscription_id)))
        .filter(e::tenant_id.eq(tenant_id))
        .filter(s::tenant_id.eq(tenant_id))
        .filter(s::customer_id.eq(customer_id))
        .filter(e::status.eq(ScheduledEventStatus::Pending))
        .filter(e::event_type.eq_any([
            ScheduledEventTypeEnum::CancelSubscription,
            ScheduledEventTypeEnum::PauseSubscription,
        ]))
        .select((e::subscription_id, e::scheduled_time))
        .load(conn)
        .await
        .attach("Error while listing scheduled subscription stops")
        .into_db_result()
}

/// Which of the components and add-ons grant their credit once (a welcome credit).
pub async fn list_once_credit_owners(
    conn: &mut PgConn,
    tenant_id: TenantId,
    components: &[SubscriptionPriceComponentId],
    add_ons: &[SubscriptionAddOnId],
) -> DbResult<(Vec<SubscriptionPriceComponentId>, Vec<SubscriptionAddOnId>)> {
    use crate::schema::subscription::dsl as s;
    use crate::schema::subscription_add_on::dsl as a;
    use crate::schema::subscription_component::dsl as c;
    use diesel::PgJsonbExpressionMethods;
    let once = serde_json::json!({"Credit": {"once": true}});
    let components = if components.is_empty() {
        Vec::new()
    } else {
        c::subscription_component
            .inner_join(s::subscription.on(s::id.eq(c::subscription_id)))
            .filter(s::tenant_id.eq(tenant_id))
            .filter(c::id.eq_any(components))
            .filter(c::legacy_fee.contains(once.clone()))
            .select(c::id)
            .load::<SubscriptionPriceComponentId>(conn)
            .await
            .attach("Error while reading one-time credit components")
            .into_db_result()?
    };
    let add_ons = if add_ons.is_empty() {
        Vec::new()
    } else {
        a::subscription_add_on
            .inner_join(s::subscription.on(s::id.eq(a::subscription_id)))
            .filter(s::tenant_id.eq(tenant_id))
            .filter(a::id.eq_any(add_ons))
            .filter(a::legacy_fee.contains(once))
            .select(a::id)
            .load::<SubscriptionAddOnId>(conn)
            .await
            .attach("Error while reading one-time credit add-ons")
            .into_db_result()?
    };
    Ok((components, add_ons))
}
