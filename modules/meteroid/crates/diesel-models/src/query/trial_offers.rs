use common_domain::ids::{
    CustomerId, PlanId, PlanVersionId, ProductFamilyId, TenantId, TrialOfferId,
};
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl, SelectableHelper};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

use crate::errors::IntoDbResult;
use crate::trial_offers::{PlanVersionPhaseRow, QuotePhaseRow, TrialOfferRow, TrialRedemptionRow};
use crate::{DbResult, PgConn};

impl TrialOfferRow {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<TrialOfferRow> {
        use crate::schema::trial_offer::dsl::trial_offer;

        diesel::insert_into(trial_offer)
            .values(self)
            .returning(TrialOfferRow::as_returning())
            .get_result(conn)
            .await
            .attach("Error while creating a trial offer")
            .into_db_result()
    }

    pub async fn list(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
    ) -> DbResult<Vec<TrialOfferRow>> {
        use crate::schema::trial_offer::dsl::{created_at, duration_days, tenant_id, trial_offer};

        trial_offer
            .filter(tenant_id.eq(tenant_id_param))
            .filter(duration_days.is_not_null())
            .order(created_at.asc())
            .select(TrialOfferRow::as_select())
            .get_results(conn)
            .await
            .attach("Error while listing trial offers")
            .into_db_result()
    }

    /// A page of the tenant's offers by id after `after`, archived ones only when asked.
    pub async fn list_after(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        after: Option<TrialOfferId>,
        include_archived: bool,
        limit: i64,
    ) -> DbResult<Vec<TrialOfferRow>> {
        use crate::schema::trial_offer::dsl::{
            archived_at, duration_days, id, tenant_id, trial_offer,
        };

        let mut query = trial_offer
            .filter(tenant_id.eq(tenant_id_param))
            .filter(duration_days.is_not_null())
            .select(TrialOfferRow::as_select())
            .order(id.asc())
            .limit(limit)
            .into_boxed();
        if let Some(after) = after {
            query = query.filter(id.gt(after));
        }
        if !include_archived {
            query = query.filter(archived_at.is_null());
        }
        query
            .get_results(conn)
            .await
            .attach("Error while paging trial offers")
            .into_db_result()
    }

    pub async fn find_by_code(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        code_param: &str,
    ) -> DbResult<Option<TrialOfferRow>> {
        use crate::schema::trial_offer::dsl::{code, tenant_id, trial_offer};

        trial_offer
            .filter(tenant_id.eq(tenant_id_param))
            .filter(code.eq(code_param))
            .select(TrialOfferRow::as_select())
            .first(conn)
            .await
            .optional()
            .attach("Error while reading a trial offer by code")
            .into_db_result()
    }

    pub async fn find(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        id_param: TrialOfferId,
    ) -> DbResult<Option<TrialOfferRow>> {
        use crate::schema::trial_offer::dsl::{id, tenant_id, trial_offer};

        trial_offer
            .filter(tenant_id.eq(tenant_id_param))
            .filter(id.eq(id_param))
            .select(TrialOfferRow::as_select())
            .first(conn)
            .await
            .optional()
            .attach("Error while reading a trial offer")
            .into_db_result()
    }

    /// The offer's terms replaced; the trials it started keep theirs (their facts).
    pub async fn update_terms(&self, conn: &mut PgConn) -> DbResult<TrialOfferRow> {
        use crate::schema::trial_offer::dsl as o;

        diesel::update(o::trial_offer)
            .filter(o::tenant_id.eq(self.tenant_id))
            .filter(o::id.eq(self.id))
            .set((
                o::name.eq(&self.name),
                o::code.eq(&self.code),
                o::duration_days.eq(self.duration_days),
                o::free.eq(self.free),
                o::trial_plan_id.eq(self.trial_plan_id),
                o::end_action.eq(&self.end_action),
                o::fallback_plan_version_id.eq(self.fallback_plan_version_id),
                o::eligibility_scope.eq(&self.eligibility_scope),
                o::card_required.eq(self.card_required),
                o::without_card.eq(&self.without_card),
                o::without_card_plan_version_id.eq(self.without_card_plan_version_id),
                o::updated_at.eq(self.updated_at),
            ))
            .returning(TrialOfferRow::as_returning())
            .get_result(conn)
            .await
            .attach("Error while updating a trial offer")
            .into_db_result()
    }

    pub async fn archive(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        id_param: TrialOfferId,
    ) -> DbResult<bool> {
        use crate::schema::trial_offer::dsl::{archived_at, id, tenant_id, trial_offer};

        diesel::update(trial_offer)
            .filter(tenant_id.eq(tenant_id_param))
            .filter(id.eq(id_param))
            .filter(archived_at.is_null())
            .set(archived_at.eq(Some(common_clock::now().naive_utc())))
            .execute(conn)
            .await
            .attach("Error while archiving a trial offer")
            .map(|updated| updated > 0)
            .into_db_result()
    }

    /// The trial of each of `plans` that names one: the offer it names, never archived, when it
    /// gives a trial.
    pub async fn of_plans(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        plans: &[PlanId],
    ) -> DbResult<Vec<(PlanId, TrialOfferRow)>> {
        use crate::schema::plan::dsl as p;
        use crate::schema::trial_offer::dsl as o;
        use diesel::{JoinOnDsl, NullableExpressionMethods};

        if plans.is_empty() {
            return Ok(Vec::new());
        }
        p::plan
            .inner_join(o::trial_offer.on(p::trial_offer_id.eq(o::id.nullable())))
            .filter(p::tenant_id.eq(tenant_id_param))
            .filter(p::id.eq_any(plans))
            .filter(o::archived_at.is_null())
            .filter(o::duration_days.is_not_null())
            .select((p::id, TrialOfferRow::as_select()))
            .get_results(conn)
            .await
            .attach("Error while reading plans' trials")
            .into_db_result()
    }

    /// The plans each of `offers` is the trial of.
    pub async fn plans_of(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        offers: &[TrialOfferId],
    ) -> DbResult<Vec<(TrialOfferId, PlanId)>> {
        use crate::schema::plan::dsl as p;

        if offers.is_empty() {
            return Ok(Vec::new());
        }
        let rows: Vec<(Option<TrialOfferId>, PlanId)> = p::plan
            .filter(p::tenant_id.eq(tenant_id_param))
            .filter(p::trial_offer_id.eq_any(offers))
            .select((p::trial_offer_id, p::id))
            .order(p::id.asc())
            .get_results(conn)
            .await
            .attach("Error while reading the plans of trial offers")
            .into_db_result()?;
        Ok(rows
            .into_iter()
            .filter_map(|(offer, plan)| offer.map(|offer| (offer, plan)))
            .collect())
    }

    /// The plan's trial set to `offer`, or none.
    pub async fn set_for_plan(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        plan_id_param: PlanId,
        offer: Option<TrialOfferId>,
    ) -> DbResult<()> {
        use crate::schema::plan::dsl as p;

        diesel::update(p::plan)
            .filter(p::tenant_id.eq(tenant_id_param))
            .filter(p::id.eq(plan_id_param))
            .set(p::trial_offer_id.eq(offer))
            .execute(conn)
            .await
            .attach("Error while setting a plan's trial")
            .map(|_| ())
            .into_db_result()
    }

    /// The offer no longer any plan's trial; the plans it was the trial of.
    pub async fn unset_on_plans(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        offer: TrialOfferId,
    ) -> DbResult<Vec<PlanId>> {
        use crate::schema::plan::dsl as p;

        diesel::update(p::plan)
            .filter(p::tenant_id.eq(tenant_id_param))
            .filter(p::trial_offer_id.eq(offer))
            .set(p::trial_offer_id.eq(None::<TrialOfferId>))
            .returning(p::id)
            .get_results(conn)
            .await
            .attach("Error while removing a trial from its plans")
            .into_db_result()
    }
}

/// What a customer is given a trial of once (R-159).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedemptionScope {
    Offer(TrialOfferId),
    Plan(PlanId),
    ProductFamily(ProductFamilyId),
}

impl TrialRedemptionRow {
    pub async fn insert_batch(conn: &mut PgConn, rows: &[TrialRedemptionRow]) -> DbResult<()> {
        use crate::schema::trial_redemption::dsl::trial_redemption;

        if rows.is_empty() {
            return Ok(());
        }
        diesel::insert_into(trial_redemption)
            .values(rows)
            .execute(conn)
            .await
            .attach("Error while recording trials given")
            .map(|_| ())
            .into_db_result()
    }

    /// How many trials of `scope` the customer was given.
    pub async fn count_given(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        customer_id_param: CustomerId,
        scope: RedemptionScope,
    ) -> DbResult<i64> {
        use crate::schema::trial_redemption::dsl as r;

        let query = r::trial_redemption
            .filter(r::tenant_id.eq(tenant_id_param))
            .filter(r::customer_id.eq(customer_id_param))
            .count()
            .into_boxed();
        let query = match scope {
            RedemptionScope::Offer(offer) => query.filter(r::trial_offer_id.eq(offer)),
            RedemptionScope::Plan(plan) => query.filter(r::plan_id.eq(plan)),
            RedemptionScope::ProductFamily(family) => query.filter(r::product_family_id.eq(family)),
        };
        query
            .get_result(conn)
            .await
            .attach("Error while counting trials given")
            .into_db_result()
    }

    pub async fn list_by_customer(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        customer_id_param: CustomerId,
    ) -> DbResult<Vec<TrialRedemptionRow>> {
        use crate::schema::trial_redemption::dsl as r;

        r::trial_redemption
            .filter(r::tenant_id.eq(tenant_id_param))
            .filter(r::customer_id.eq(customer_id_param))
            .order(r::created_at.asc())
            .select(TrialRedemptionRow::as_select())
            .load(conn)
            .await
            .attach("Error while listing trials given")
            .into_db_result()
    }
}

impl PlanVersionPhaseRow {
    /// The plan version's template replaced by `rows`.
    pub async fn replace_for_version(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        plan_version_id_param: PlanVersionId,
        rows: &[PlanVersionPhaseRow],
    ) -> DbResult<()> {
        use crate::schema::plan_version_phase::dsl::{
            plan_version_id, plan_version_phase, tenant_id,
        };

        diesel::delete(plan_version_phase)
            .filter(tenant_id.eq(tenant_id_param))
            .filter(plan_version_id.eq(plan_version_id_param))
            .execute(conn)
            .await
            .attach("Error while replacing a plan version's phases")
            .into_db_result()?;
        if rows.is_empty() {
            return Ok(());
        }
        diesel::insert_into(plan_version_phase)
            .values(rows)
            .execute(conn)
            .await
            .attach("Error while writing a plan version's phases")
            .map(|_| ())
            .into_db_result()
    }

    pub async fn list_for_version(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        plan_version_id_param: PlanVersionId,
    ) -> DbResult<Vec<PlanVersionPhaseRow>> {
        use crate::schema::plan_version_phase::dsl::{
            plan_version_id, plan_version_phase, seq, tenant_id,
        };

        plan_version_phase
            .filter(tenant_id.eq(tenant_id_param))
            .filter(plan_version_id.eq(plan_version_id_param))
            .order(seq.asc())
            .select(PlanVersionPhaseRow::as_select())
            .get_results(conn)
            .await
            .attach("Error while listing a plan version's phases")
            .into_db_result()
    }
}

impl QuotePhaseRow {
    /// The quote's phases replaced by `rows`.
    pub async fn replace_for_quote(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        quote_id_param: common_domain::ids::QuoteId,
        rows: &[QuotePhaseRow],
    ) -> DbResult<()> {
        use crate::schema::quote_phase::dsl::{quote_id, quote_phase, tenant_id};

        diesel::delete(quote_phase)
            .filter(tenant_id.eq(tenant_id_param))
            .filter(quote_id.eq(quote_id_param))
            .execute(conn)
            .await
            .attach("Error while replacing a quote's phases")
            .into_db_result()?;
        if rows.is_empty() {
            return Ok(());
        }
        diesel::insert_into(quote_phase)
            .values(rows)
            .execute(conn)
            .await
            .attach("Error while writing a quote's phases")
            .map(|_| ())
            .into_db_result()
    }

    pub async fn list_for_quote(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        quote_id_param: common_domain::ids::QuoteId,
    ) -> DbResult<Vec<QuotePhaseRow>> {
        use crate::schema::quote_phase::dsl::{quote_id, quote_phase, seq, tenant_id};

        quote_phase
            .filter(tenant_id.eq(tenant_id_param))
            .filter(quote_id.eq(quote_id_param))
            .order(seq.asc())
            .select(QuotePhaseRow::as_select())
            .get_results(conn)
            .await
            .attach("Error while listing a quote's phases")
            .into_db_result()
    }
}
