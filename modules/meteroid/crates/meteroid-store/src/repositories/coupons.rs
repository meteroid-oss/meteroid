use crate::domain::Customer;
use crate::domain::coupons::{
    Coupon, CouponDiscount, CouponFilter, CouponNew, CouponPatch, CouponStatusPatch, CouponTerms,
    CouponValidationError, fixed_amounts_to_row, validate_duration_months, validate_fixed_amounts,
};
use crate::domain::eligibility::{self, CustomerFacts, Eligibility};
use crate::domain::entity_activity::Actor;
use crate::domain::outbox_event::OutboxEvent;
use crate::domain::{AppliedCouponForDisplay, PaginatedVec, PaginationRequest};
use crate::errors::StoreError;
use crate::store::PgConn;
use crate::{Store, StoreResult};
use common_domain::ids::{AddOnId, CouponId, CustomerId, PlanId, TenantId};
use diesel_models::add_ons::AddOnRow;
use diesel_models::applied_coupons::{AppliedCouponForDisplayRow, AppliedCouponRow};
use diesel_models::coupon_add_ons::{CouponAddOnRow, CouponAddOnRowNew};
use diesel_models::coupon_plans::{CouponPlanRow, CouponPlanRowNew};
use diesel_models::coupons::{CouponRow, CouponRowNew, CouponRowPatch, CouponStatusRowPatch};
use diesel_models::plans::PlanRow;
use diesel_models::query::coupons::CodeMatch;
use diesel_models::quote_coupons::QuoteCouponRow;
use error_stack::Report;
use meteroid_store_macros::with_conn_delegate;
use scoped_futures::ScopedFutureExt;

#[with_conn_delegate]
#[async_trait::async_trait]
pub trait CouponInterface {
    async fn list_coupons(
        &self,
        tenant_id: TenantId,
        pagination: PaginationRequest,
        search: Option<String>,
        filter: CouponFilter,
        order_by: Option<String>,
        plan_id: Option<PlanId>,
    ) -> StoreResult<PaginatedVec<Coupon>>;
    #[delegated]
    async fn get_coupon_by_id(&self, tenant_id: TenantId, id: CouponId) -> StoreResult<Coupon>;

    async fn create_coupon(&self, actor: Actor, coupon: CouponNew) -> StoreResult<Coupon>;
    /// `create_coupon` with who may redeem it, from when, with what add-ons, and a fixed
    /// discount's amounts in other currencies.
    async fn create_coupon_with_terms(
        &self,
        actor: Actor,
        coupon: CouponNew,
        terms: CouponTerms,
    ) -> StoreResult<Coupon>;
    async fn delete_coupon(&self, tenant_id: TenantId, id: CouponId) -> StoreResult<()>;
    async fn update_coupon(&self, actor: Actor, coupon: CouponPatch) -> StoreResult<Coupon>;
    async fn update_coupon_status(
        &self,
        actor: Actor,
        coupon: CouponStatusPatch,
    ) -> StoreResult<Coupon>;

    async fn list_applied_coupons_by_coupon_id(
        &self,
        tenant_id: TenantId,
        coupon_id: CouponId,
        pagination: PaginationRequest,
    ) -> StoreResult<PaginatedVec<AppliedCouponForDisplay>>;

    async fn list_coupons_by_codes(
        &self,
        tenant_id: TenantId,
        codes: &[String],
    ) -> StoreResult<Vec<Coupon>>;

    async fn list_coupons_by_codes_tx(
        &self,
        conn: &mut diesel_models::PgConn,
        tenant_id: TenantId,
        codes: &[String],
    ) -> StoreResult<Vec<Coupon>>;

    /// The coupon each code names, in order and without repeats; refuses the request, listing
    /// them, when a code names no live coupon or only coupons that differ from it in case.
    async fn resolve_coupon_codes(
        &self,
        tenant_id: TenantId,
        codes: &[String],
    ) -> StoreResult<Vec<CouponId>>;

    /// A page of the tenant's coupons by keyset (API v2), with their scope: those of `filter`
    /// after `after`, in id order, at most `limit`. No filter: those not archived.
    async fn list_coupons_after(
        &self,
        tenant_id: TenantId,
        filter: Option<CouponFilter>,
        after: Option<CouponId>,
        limit: usize,
    ) -> StoreResult<Vec<Coupon>>;

    /// The newest archived coupon with exactly `code`, its code being free for live ones.
    async fn find_archived_coupon_by_code(
        &self,
        tenant_id: TenantId,
        code: &str,
    ) -> StoreResult<Option<Coupon>>;
}

/// Refuses, by name, the codes naming no live coupon or several (see [`CodeMatch::of`]).
pub(crate) fn match_coupon_codes(
    codes: &[String],
    rows: &[CouponRow],
) -> StoreResult<Vec<CouponId>> {
    let mut ids: Vec<CouponId> = Vec::new();
    let mut unknown = Vec::new();
    let mut ambiguous = Vec::new();
    for requested in codes {
        match CodeMatch::of(requested, rows) {
            CodeMatch::Found(row) if !ids.contains(&row.id) => ids.push(row.id),
            CodeMatch::Found(_) => {}
            CodeMatch::Unknown => unknown.push(requested.as_str()),
            CodeMatch::Ambiguous => ambiguous.push(requested.as_str()),
        }
    }
    let mut problems = Vec::new();
    if !unknown.is_empty() {
        problems.push(format!("unknown coupon codes: {}", unknown.join(", ")));
    }
    if !ambiguous.is_empty() {
        problems.push(format!(
            "coupon codes matching several coupons that differ only in case, send the exact code: {}",
            ambiguous.join(", ")
        ));
    }
    if problems.is_empty() {
        Ok(ids)
    } else {
        Err(Report::new(StoreError::InvalidArgument(
            problems.join("; "),
        )))
    }
}

/// Longest coupon code: what the portal accepts at redemption.
pub const MAX_COUPON_CODE_LENGTH: usize = 64;

/// `rows` as coupons, with the plans and add-ons they are scoped to.
pub(crate) async fn coupons_with_scope(
    conn: &mut PgConn,
    rows: Vec<CouponRow>,
) -> StoreResult<Vec<Coupon>> {
    let coupon_ids: Vec<CouponId> = rows.iter().map(|c| c.id).collect();
    let plan_ids_map = CouponPlanRow::list_by_coupon_ids(conn, &coupon_ids)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    let add_on_ids_map = CouponAddOnRow::list_by_coupon_ids(conn, &coupon_ids)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    rows.into_iter()
        .map(|row| {
            let id = row.id;
            let mut coupon: Coupon = row.try_into()?;
            coupon.plan_ids = plan_ids_map.get(&id).cloned().unwrap_or_default();
            coupon.add_on_ids = add_on_ids_map.get(&id).cloned().unwrap_or_default();
            Ok(coupon)
        })
        .collect()
}

async fn coupon_with_scope(conn: &mut PgConn, row: CouponRow) -> StoreResult<Coupon> {
    coupons_with_scope(conn, vec![row])
        .await?
        .pop()
        .ok_or_else(|| Report::new(StoreError::ValueNotFound("coupon".to_owned())))
}

/// A rule as stored: normalized, refused when unreadable, none when it lets anyone in.
fn stored_rule(rule: Option<Eligibility>) -> StoreResult<Option<Eligibility>> {
    let Some(rule) = rule.map(Eligibility::normalized) else {
        return Ok(None);
    };
    rule.validate()
        .map_err(|message| Report::new(StoreError::InvalidArgument(message)))?;
    Ok((!rule.is_open()).then_some(rule))
}

fn refuse_if_valid_after_expiry(
    valid_from: Option<chrono::NaiveDateTime>,
    expires_at: Option<chrono::NaiveDateTime>,
) -> StoreResult<()> {
    if let (Some(from), Some(until)) = (valid_from, expires_at)
        && from >= until
    {
        return Err(Report::new(StoreError::InvalidArgument(
            "a coupon must become valid before it expires".to_owned(),
        )));
    }
    Ok(())
}

/// A subscription a coupon is redeemed on, as the checks of a redemption read it.
pub(crate) struct Redemption<'a> {
    pub customer_id: CustomerId,
    pub currency: &'a str,
    pub plan_id: Option<PlanId>,
    pub add_on_ids: &'a [AddOnId],
}

/// Refuses the first of `coupons` `redemption` cannot take at `at`: not valid yet, a fixed
/// discount with no amount in its currency or a credit in another, the subscription out of its
/// scope, or its rule leaving the customer out.
pub(crate) async fn refuse_unredeemable(
    conn: &mut PgConn,
    tenant_id: TenantId,
    coupons: &[Coupon],
    redemption: &Redemption<'_>,
    at: chrono::NaiveDateTime,
) -> StoreResult<()> {
    let refused = |message: String| Err(Report::new(StoreError::InvalidArgument(message)));
    for coupon in coupons {
        if !coupon.is_valid_yet(at) {
            return refused(CouponValidationError::NotYetValid.to_error_message(&coupon.code));
        }
        match &coupon.discount {
            CouponDiscount::Fixed { .. }
                if coupon.fixed_amount_in(redemption.currency).is_none() =>
            {
                return refused(
                    CouponValidationError::NoAmountIn(redemption.currency.to_owned())
                        .to_error_message(&coupon.code),
                );
            }
            CouponDiscount::Credit { currency, .. } if currency != redemption.currency => {
                return refused(
                    CouponValidationError::CurrencyMismatch {
                        coupon_currency: currency.clone(),
                        expected: redemption.currency.to_owned(),
                    }
                    .to_error_message(&coupon.code),
                );
            }
            _ => {}
        }
        if !coupon.applies_to(redemption.plan_id, redemption.add_on_ids) {
            return refused(format!(
                "Coupon {} cannot be applied to this plan",
                coupon.code
            ));
        }
    }

    refuse_ineligible(conn, tenant_id, coupons, redemption.customer_id).await
}

/// Refuses the first of `coupons` whose rule leaves the customer out, with the message they are
/// shown.
pub(crate) async fn refuse_ineligible(
    conn: &mut PgConn,
    tenant_id: TenantId,
    coupons: &[Coupon],
    customer_id: CustomerId,
) -> StoreResult<()> {
    let ruled: Vec<&Eligibility> = coupons
        .iter()
        .filter_map(|coupon| coupon.eligibility.as_ref())
        .collect();
    if ruled.is_empty() {
        return Ok(());
    }
    let customer: Customer =
        diesel_models::customers::CustomerRow::find_by_id(conn, &customer_id, &tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .try_into()?;
    let facts = CustomerFacts::of(&customer);
    for rule in ruled {
        eligibility::check(rule, &facts)
            .map_err(|why| Report::new(StoreError::InvalidArgument(why.message())))?;
    }
    Ok(())
}

/// The currencies a fixed discount has an amount in.
fn fixed_currencies(
    discount: &CouponDiscount,
    amounts: &std::collections::BTreeMap<String, rust_decimal::Decimal>,
) -> Vec<String> {
    match discount {
        CouponDiscount::Fixed { currency, .. } => std::iter::once(currency.clone())
            .chain(amounts.keys().cloned())
            .collect(),
        _ => Vec::new(),
    }
}

/// Refuses taking away a fixed discount's amount in a currency it is actively redeemed in.
async fn refuse_removing_redeemed_amounts(
    conn: &mut PgConn,
    current: &Coupon,
    discount: &CouponDiscount,
    amounts: &std::collections::BTreeMap<String, rust_decimal::Decimal>,
) -> StoreResult<()> {
    let kept = fixed_currencies(discount, amounts);
    let removed: Vec<String> = fixed_currencies(&current.discount, &current.fixed_amounts)
        .into_iter()
        .filter(|currency| !kept.contains(currency))
        .collect();
    if removed.is_empty() {
        return Ok(());
    }
    let redeemed = AppliedCouponRow::active_currencies_of_coupon(conn, current.id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    match removed.iter().find(|currency| redeemed.contains(currency)) {
        Some(currency) => Err(Report::new(StoreError::InvalidArgument(format!(
            "the coupon is redeemed on active subscriptions in {currency}: its amount in \
             {currency} cannot be removed"
        )))),
        None => Ok(()),
    }
}

/// Refuses plans that are not the tenant's: a coupon is scoped to its own tenant's plans only.
async fn refuse_foreign_plans(
    conn: &mut PgConn,
    tenant_id: TenantId,
    plan_ids: &[PlanId],
) -> StoreResult<()> {
    if plan_ids.is_empty() {
        return Ok(());
    }
    let found = PlanRow::list_by_ids(conn, plan_ids, tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    match plan_ids
        .iter()
        .find(|id| !found.iter().any(|row| row.id == **id))
    {
        Some(unknown) => Err(Report::new(StoreError::InvalidArgument(format!(
            "plan {unknown} is not one of this tenant's plans"
        )))),
        None => Ok(()),
    }
}

/// Scopes the coupon to `add_on_ids`, each one of the tenant's: a live add-on, or one it is
/// scoped to already.
async fn replace_coupon_add_ons(
    conn: &mut PgConn,
    tenant_id: TenantId,
    coupon_id: CouponId,
    add_on_ids: &[AddOnId],
) -> StoreResult<()> {
    let mut add_on_ids = add_on_ids.to_vec();
    add_on_ids.sort();
    add_on_ids.dedup();
    let current = CouponAddOnRow::list_by_coupon_ids(conn, &[coupon_id])
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .remove(&coupon_id)
        .unwrap_or_default();
    let added: Vec<AddOnId> = add_on_ids
        .iter()
        .filter(|id| !current.contains(id))
        .copied()
        .collect();
    if !added.is_empty() {
        let found = AddOnRow::list_by_ids(conn, &added, &tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        if let Some(unknown) = added
            .iter()
            .find(|id| !found.iter().any(|row| row.id == **id))
        {
            return Err(Report::new(StoreError::InvalidArgument(format!(
                "add-on {unknown} is not one of this tenant's add-ons"
            ))));
        }
    }
    CouponAddOnRow::delete_by_coupon_id(conn, coupon_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    let rows: Vec<CouponAddOnRowNew> = add_on_ids
        .iter()
        .map(|add_on_id| CouponAddOnRowNew {
            coupon_id,
            add_on_id: *add_on_id,
        })
        .collect();
    CouponAddOnRowNew::insert_batch(&rows, conn)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    Ok(())
}

/// A quote applies its coupons as they are at conversion, and its signers agreed to the terms
/// they were shown.
async fn refuse_if_on_open_quotes(
    conn: &mut PgConn,
    tenant_id: TenantId,
    coupon_id: CouponId,
) -> StoreResult<()> {
    const LISTED: usize = 3;
    let numbers = QuoteCouponRow::list_open_quote_numbers_by_coupon_id(
        conn,
        tenant_id,
        coupon_id,
        LISTED as i64 + 1,
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)?;
    if numbers.is_empty() {
        return Ok(());
    }
    let mut listed = numbers[..numbers.len().min(LISTED)].join(", ");
    if numbers.len() > LISTED {
        listed.push_str(" and others");
    }
    Err(Report::new(StoreError::InvalidArgument(format!(
        "the discount of a coupon cannot change while open quotes offer it: {listed}"
    ))))
}

/// The code as stored. Every code is 1 to 64 characters with no surrounding whitespace. A
/// promotional-credit code is also limited to letters, digits, `-` and `_`, never reusable, and
/// stored upper-case. Invoice discount codes keep their exact case. Every code is looked up
/// whatever the case of its ASCII letters (`CodeMatch`), like a Stripe promotion code.
fn normalize_coupon_code(coupon: &CouponNew) -> StoreResult<String> {
    let invalid = |message: &str| Err(Report::new(StoreError::InvalidArgument(message.to_owned())));
    let code = coupon.code.as_str();
    if code.is_empty() || code.trim() != code {
        return invalid("a coupon code must not be empty or start or end with whitespace");
    }
    if code.chars().count() > MAX_COUPON_CODE_LENGTH {
        return invalid("a coupon code is at most 64 characters");
    }
    if !coupon.discount.is_credit() {
        return Ok(code.to_owned());
    }
    if !code
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return invalid(
            "a promotional credit code contains only letters, digits, hyphens and underscores",
        );
    }
    if coupon.reusable {
        return invalid(
            "a promotional credit coupon is redeemed once per customer, so it cannot be reusable",
        );
    }
    Ok(code.to_ascii_uppercase())
}

#[async_trait::async_trait]
impl CouponInterface for Store {
    async fn list_coupons(
        &self,
        tenant_id: TenantId,
        pagination: PaginationRequest,
        search: Option<String>,
        filter: CouponFilter,
        order_by: Option<String>,
        plan_id: Option<PlanId>,
    ) -> StoreResult<PaginatedVec<Coupon>> {
        let mut conn = self.get_conn().await?;

        let coupons = CouponRow::list_by_tenant_id(
            &mut conn,
            tenant_id,
            pagination.into(),
            search,
            filter.into(),
            order_by.as_deref(),
            plan_id,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let items = coupons_with_scope(&mut conn, coupons.items).await?;

        Ok(PaginatedVec {
            items,
            total_pages: coupons.total_pages,
            total_results: coupons.total_results,
        })
    }

    async fn get_coupon_by_id_with_conn(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: CouponId,
    ) -> StoreResult<Coupon> {
        let row = CouponRow::get_by_id(conn, tenant_id, id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        coupon_with_scope(conn, row).await
    }

    async fn create_coupon(&self, actor: Actor, coupon: CouponNew) -> StoreResult<Coupon> {
        self.create_coupon_with_terms(actor, coupon, CouponTerms::default())
            .await
    }

    async fn create_coupon_with_terms(
        &self,
        actor: Actor,
        mut coupon: CouponNew,
        terms: CouponTerms,
    ) -> StoreResult<Coupon> {
        let mut conn = self.get_conn().await?;

        coupon.discount.validate()?;
        validate_fixed_amounts(&coupon.discount, &terms.fixed_amounts)?;
        refuse_if_valid_after_expiry(terms.valid_from, coupon.expires_at)?;
        let eligibility = stored_rule(terms.eligibility)?;
        if coupon.discount.is_credit()
            && (coupon.recurring_value.is_some() || terms.duration_months.is_some())
        {
            return Err(Report::new(StoreError::InvalidArgument(
                "a promotional credit coupon must not have a duration".to_owned(),
            )));
        }
        validate_duration_months(terms.duration_months, coupon.recurring_value)?;
        coupon.code = normalize_coupon_code(&coupon)?;
        let is_credit = coupon.discount.is_credit();
        let tenant_id = coupon.tenant_id;
        let plan_ids = coupon.plan_ids.clone();
        let add_on_ids = terms.add_on_ids;
        let mut coupon_row: CouponRowNew = coupon.try_into()?;
        coupon_row.eligibility = Eligibility::to_row(eligibility.as_ref());
        coupon_row.valid_from = terms.valid_from;
        coupon_row.fixed_amounts = fixed_amounts_to_row(&terms.fixed_amounts)?;
        coupon_row.duration_months = terms.duration_months;
        let coupon_id = coupon_row.id;

        let coupon = self
            .transaction_with(&mut conn, |conn| {
                let actor = &actor;
                async move {
                    // Credit codes match whatever the case, so they must not shadow, nor be
                    // shadowed by, another live code that differs only in case.
                    if CouponRow::code_taken_ignoring_case(
                        conn,
                        tenant_id,
                        &coupon_row.code,
                        !is_credit,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
                    {
                        return Err(Report::new(StoreError::DuplicateValue {
                            entity: "coupon",
                            key: Some(coupon_row.code.clone()),
                        }));
                    }
                    refuse_foreign_plans(conn, tenant_id, &plan_ids).await?;
                    let row = coupon_row
                        .insert(conn)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    if !plan_ids.is_empty() {
                        let plan_rows: Vec<CouponPlanRowNew> = plan_ids
                            .iter()
                            .map(|plan_id| CouponPlanRowNew {
                                coupon_id,
                                plan_id: *plan_id,
                            })
                            .collect();
                        CouponPlanRowNew::insert_batch(&plan_rows, conn)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                    }

                    replace_coupon_add_ons(conn, tenant_id, coupon_id, &add_on_ids).await?;

                    let mut coupon: Coupon = row.try_into()?;
                    coupon.plan_ids = plan_ids;
                    coupon.add_on_ids = add_on_ids;

                    self.internal
                        .record_outbox_batch_tx(
                            conn,
                            tenant_id,
                            actor,
                            vec![OutboxEvent::coupon_created(coupon.clone().into())],
                        )
                        .await?;

                    Ok(coupon)
                }
                .scope_boxed()
            })
            .await?;

        Ok(coupon)
    }

    async fn delete_coupon(&self, tenant_id: TenantId, id: CouponId) -> StoreResult<()> {
        // Deleting a coupon cascades to the applied coupons of live subscriptions.
        self.transaction_bumping_tenant(tenant_id, |conn| {
            async move {
                CouponRow::delete(conn, tenant_id, id).await?;
                Ok(())
            }
            .scope_boxed()
        })
        .await
    }

    async fn update_coupon(&self, actor: Actor, mut coupon: CouponPatch) -> StoreResult<Coupon> {
        let tenant_id = coupon.tenant_id;
        let coupon_id = coupon.id;
        let plan_ids = coupon.plan_ids.clone();
        let add_on_ids = coupon.add_on_ids.take();
        let new_discount = coupon.discount.clone();
        if let Some(discount) = &new_discount {
            discount.validate()?;
        }
        if let Some(rule) = coupon.eligibility.take() {
            coupon.eligibility = Some(stored_rule(rule)?);
        }

        self.transaction_bumping_tenant(tenant_id, |conn| {
            let actor = &actor;
            async move {
                let mut coupon = coupon;
                let current: Coupon = CouponRow::lock_by_id(conn, tenant_id, coupon_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
                    .try_into()?;
                if let Some(discount) = &new_discount {
                    // Past redemptions were granted credit or discounted, never both.
                    if current.discount.is_credit() != discount.is_credit() {
                        return Err(Report::new(StoreError::InvalidArgument(
                            "a coupon cannot switch between promotional credit and an invoice discount"
                                .to_owned(),
                        )));
                    }
                }
                let discount = new_discount.as_ref().unwrap_or(&current.discount);
                // Amounts in other currencies belong to a fixed discount only.
                if !matches!(discount, CouponDiscount::Fixed { .. })
                    && coupon.fixed_amounts.is_none()
                    && !current.fixed_amounts.is_empty()
                {
                    coupon.fixed_amounts = Some(Default::default());
                }
                let fixed_amounts = coupon
                    .fixed_amounts
                    .as_ref()
                    .unwrap_or(&current.fixed_amounts);
                validate_fixed_amounts(discount, fixed_amounts)?;
                refuse_removing_redeemed_amounts(conn, &current, discount, fixed_amounts).await?;
                // Redemptions keep the months they were made with: an edit reaches later ones only.
                // A coupon counting invoices keeps counting them; promotional credit lasts none.
                if let Some(months) = coupon.duration_months {
                    if months.is_some()
                        && (current.recurring_value.is_some() || discount.is_credit())
                    {
                        return Err(Report::new(StoreError::InvalidArgument(
                            "only a discount lasting forever or months can last months".to_owned(),
                        )));
                    }
                    validate_duration_months(months, None)?;
                }
                let months_changed = coupon
                    .duration_months
                    .is_some_and(|months| months != current.duration_months);
                if *discount != current.discount
                    || *fixed_amounts != current.fixed_amounts
                    || months_changed
                {
                    refuse_if_on_open_quotes(conn, tenant_id, coupon_id).await?;
                }
                refuse_if_valid_after_expiry(
                    coupon.valid_from.unwrap_or(current.valid_from),
                    coupon.expires_at.unwrap_or(current.expires_at),
                )?;
                let coupon_row: CouponRowPatch = coupon.try_into()?;
                let row = coupon_row
                    .patch(conn)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                let final_plan_ids = if let Some(new_plan_ids) = plan_ids {
                    refuse_foreign_plans(conn, tenant_id, &new_plan_ids).await?;
                    CouponPlanRow::delete_by_coupon_id(conn, coupon_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    if !new_plan_ids.is_empty() {
                        let plan_rows: Vec<CouponPlanRowNew> = new_plan_ids
                            .iter()
                            .map(|plan_id| CouponPlanRowNew {
                                coupon_id,
                                plan_id: *plan_id,
                            })
                            .collect();
                        CouponPlanRowNew::insert_batch(&plan_rows, conn)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                    }
                    new_plan_ids
                } else {
                    CouponPlanRow::list_by_coupon_id(conn, coupon_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                };

                if let Some(add_on_ids) = &add_on_ids {
                    replace_coupon_add_ons(conn, tenant_id, coupon_id, add_on_ids).await?;
                }

                let mut coupon = coupon_with_scope(conn, row).await?;
                coupon.plan_ids = final_plan_ids;

                self.internal
                    .record_outbox_batch_tx(
                        conn,
                        tenant_id,
                        actor,
                        vec![OutboxEvent::coupon_updated(coupon.clone().into())],
                    )
                    .await?;

                Ok(coupon)
            }
            .scope_boxed()
        })
        .await
    }

    async fn update_coupon_status(
        &self,
        actor: Actor,
        coupon: CouponStatusPatch,
    ) -> StoreResult<Coupon> {
        let mut conn = self.get_conn().await?;

        let tenant_id = coupon.tenant_id;
        let is_archiving = coupon.archived_at.as_ref().is_some_and(|v| v.is_some());
        let coupon_row: CouponStatusRowPatch = coupon.into();

        self.transaction_with(&mut conn, |conn| {
            let actor = &actor;
            async move {
                let row = coupon_row
                    .patch(conn)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                let coupon = coupon_with_scope(conn, row).await?;

                if is_archiving {
                    self.internal
                        .record_outbox_batch_tx(
                            conn,
                            tenant_id,
                            actor,
                            vec![OutboxEvent::coupon_archived(coupon.clone().into())],
                        )
                        .await?;
                }

                Ok(coupon)
            }
            .scope_boxed()
        })
        .await
    }

    async fn list_applied_coupons_by_coupon_id(
        &self,
        tenant_id: TenantId,
        coupon_id: CouponId,
        pagination: PaginationRequest,
    ) -> StoreResult<PaginatedVec<AppliedCouponForDisplay>> {
        let mut conn = self.get_conn().await?;

        let coupons = AppliedCouponForDisplayRow::list_by_coupon_id(
            &mut conn,
            &coupon_id,
            &tenant_id,
            pagination.into(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        Ok(PaginatedVec {
            items: coupons
                .items
                .into_iter()
                .map(std::convert::Into::into)
                .collect::<Vec<_>>(),
            total_pages: coupons.total_pages,
            total_results: coupons.total_results,
        })
    }

    async fn list_coupons_by_codes_tx(
        &self,
        conn: &mut diesel_models::PgConn,
        tenant_id: TenantId,
        codes: &[String],
    ) -> StoreResult<Vec<Coupon>> {
        let rows = CouponRow::list_by_codes(conn, tenant_id, codes)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        coupons_with_scope(conn, rows).await
    }

    async fn list_coupons_by_codes(
        &self,
        tenant_id: TenantId,
        codes: &[String],
    ) -> StoreResult<Vec<Coupon>> {
        let mut conn = self.get_conn().await?;

        self.list_coupons_by_codes_tx(&mut conn, tenant_id, codes)
            .await
    }

    async fn resolve_coupon_codes(
        &self,
        tenant_id: TenantId,
        codes: &[String],
    ) -> StoreResult<Vec<CouponId>> {
        let mut conn = self.get_conn().await?;
        let rows = CouponRow::candidates_by_codes(&mut conn, tenant_id, codes)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        match_coupon_codes(codes, &rows)
    }

    async fn list_coupons_after(
        &self,
        tenant_id: TenantId,
        filter: Option<CouponFilter>,
        after: Option<CouponId>,
        limit: usize,
    ) -> StoreResult<Vec<Coupon>> {
        let mut conn = self.get_conn().await?;
        let rows = CouponRow::list_page(
            &mut conn,
            tenant_id,
            filter.map(Into::into),
            after,
            i64::try_from(limit).unwrap_or(i64::MAX),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
        coupons_with_scope(&mut conn, rows).await
    }

    async fn find_archived_coupon_by_code(
        &self,
        tenant_id: TenantId,
        code: &str,
    ) -> StoreResult<Option<Coupon>> {
        let mut conn = self.get_conn().await?;
        match CouponRow::find_archived_by_code(&mut conn, tenant_id, code)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
        {
            Some(row) => Ok(Some(coupon_with_scope(&mut conn, row).await?)),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common_domain::ids::BaseId;

    fn row(code: &str) -> CouponRow {
        let now = common_clock::now().naive_utc();
        CouponRow {
            id: CouponId::new(),
            code: code.to_owned(),
            description: String::new(),
            tenant_id: TenantId::new(),
            discount: serde_json::json!({}),
            expires_at: None,
            redemption_limit: None,
            recurring_value: None,
            reusable: true,
            created_at: now,
            updated_at: now,
            redemption_count: 0,
            last_redemption_at: None,
            disabled: false,
            archived_at: None,
            eligibility: None,
            valid_from: None,
            fixed_amounts: None,
            duration_months: None,
        }
    }

    fn codes(codes: &[&str]) -> Vec<String> {
        codes.iter().map(|code| (*code).to_owned()).collect()
    }

    #[test]
    fn codes_match_ignoring_case_and_an_exact_code_wins() {
        let rows = [row("Summer10"), row("WELCOME-25")];
        let (summer, welcome) = (rows[0].id, rows[1].id);
        assert_eq!(
            match_coupon_codes(&codes(&["summer10", " welcome-25 ", "SUMMER10"]), &rows).unwrap(),
            vec![summer, welcome]
        );
        let rows = [row("Summer10"), row("SUMMER10")];
        assert_eq!(
            match_coupon_codes(&codes(&["SUMMER10", "Summer10"]), &rows).unwrap(),
            vec![rows[1].id, rows[0].id]
        );
    }

    /// Codes stored with surrounding spaces before codes were validated are matched by the code
    /// itself; one sent as stored still names it first.
    #[test]
    fn a_code_stored_with_surrounding_spaces_is_matched_by_the_code() {
        let rows = [row(" SAVE10 "), row("Summer10")];
        let (save, summer) = (rows[0].id, rows[1].id);
        assert_eq!(
            match_coupon_codes(
                &codes(&["SAVE10", " SAVE10 ", "save10", " summer10"]),
                &rows
            )
            .unwrap(),
            vec![save, summer]
        );
        let rows = [row("SAVE10 "), row(" SAVE10")];
        assert_eq!(
            match_coupon_codes(&codes(&["SAVE10 ", " SAVE10"]), &rows).unwrap(),
            vec![rows[0].id, rows[1].id]
        );
        assert!(match_coupon_codes(&codes(&["SAVE10"]), &rows).is_err());
        let rows = [row("SAVE10"), row(" SAVE10 ")];
        assert_eq!(
            match_coupon_codes(&codes(&[" SAVE10"]), &rows).unwrap(),
            vec![rows[0].id]
        );
    }

    #[test]
    fn non_ascii_codes_match_exactly_and_fold_only_ascii_letters() {
        let rows = [row("Straße"), row("été-10")];
        let (strasse, ete) = (rows[0].id, rows[1].id);
        assert_eq!(
            match_coupon_codes(&codes(&["Straße", "été-10", "STRAßE", "straße"]), &rows).unwrap(),
            vec![strasse, ete]
        );
        assert!(match_coupon_codes(&codes(&["ÉTÉ-10"]), &rows).is_err());
        assert!(match_coupon_codes(&codes(&["STRASSE"]), &rows).is_err());
    }

    #[test]
    fn a_fixed_discount_has_amounts_in_its_own_currency_and_the_others() {
        let usd = CouponDiscount::Fixed {
            currency: "USD".to_owned(),
            amount: rust_decimal::Decimal::from(10),
        };
        let amounts =
            std::collections::BTreeMap::from([("EUR".to_owned(), rust_decimal::Decimal::from(9))]);
        assert_eq!(fixed_currencies(&usd, &amounts), vec!["USD", "EUR"]);
        let percentage = CouponDiscount::Percentage(rust_decimal::Decimal::from(10));
        assert!(fixed_currencies(&percentage, &amounts).is_empty());
    }

    #[test]
    fn unknown_and_ambiguous_codes_are_refused_by_name() {
        let rows = [row("Summer10"), row("SUMMER10")];
        let refused =
            match_coupon_codes(&codes(&["NOPE", "summer10", "Gone"]), &rows).expect_err("refused");
        match refused.current_context() {
            StoreError::InvalidArgument(message) => assert_eq!(
                message,
                "unknown coupon codes: NOPE, Gone; coupon codes matching several coupons that \
                 differ only in case, send the exact code: summer10"
            ),
            other => panic!("expected an invalid argument, got {other:?}"),
        }
    }
}
