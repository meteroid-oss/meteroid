use chrono::NaiveDateTime;
use common_domain::ids::{
    BaseId, CouponId, CustomerId, OfferApplicationId, OfferGrantId, OfferId, SubscriptionId,
    TenantId,
};
use diesel::{
    BoolExpressionMethods, ExpressionMethods, NullableExpressionMethods, OptionalExtension,
    QueryDsl, SelectableHelper,
};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

use crate::coupons::CouponRow;
use crate::errors::IntoDbResult;
use crate::offers::{OfferApplicationItemRow, OfferApplicationRow, OfferGrantRow};
use crate::trial_offers::TrialOfferRow;
use crate::{DbResult, PgConn};

impl TrialOfferRow {
    /// The offer's terms and name replaced, its trial included (`duration_days` `None`: none).
    pub async fn update_offer(&self, conn: &mut PgConn) -> DbResult<TrialOfferRow> {
        use crate::schema::trial_offer::dsl as o;

        diesel::update(o::trial_offer)
            .filter(o::tenant_id.eq(self.tenant_id))
            .filter(o::id.eq(self.id))
            .set((
                o::name.eq(&self.name),
                o::description.eq(&self.description),
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
            .attach("Error while updating an offer")
            .into_db_result()
    }

    /// A page of the tenant's catalog offers by id after `after`, archived ones only when asked.
    pub async fn list_standalone_after(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        after: Option<OfferId>,
        include_archived: bool,
        limit: i64,
    ) -> DbResult<Vec<TrialOfferRow>> {
        use crate::schema::trial_offer::dsl as o;

        let mut query = o::trial_offer
            .filter(o::tenant_id.eq(tenant_id_param))
            .filter(o::standalone.eq(true))
            .select(TrialOfferRow::as_select())
            .order(o::id.asc())
            .limit(limit)
            .into_boxed();
        if let Some(after) = after {
            query = query.filter(o::id.gt(after));
        }
        if !include_archived {
            query = query.filter(o::archived_at.is_null());
        }
        query
            .get_results(conn)
            .await
            .attach("Error while paging offers")
            .into_db_result()
    }

    pub async fn list_by_ids(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        ids: &[OfferId],
    ) -> DbResult<Vec<TrialOfferRow>> {
        use crate::schema::trial_offer::dsl as o;

        if ids.is_empty() {
            return Ok(Vec::new());
        }
        o::trial_offer
            .filter(o::tenant_id.eq(tenant_id_param))
            .filter(o::id.eq_any(ids))
            .select(TrialOfferRow::as_select())
            .get_results(conn)
            .await
            .attach("Error while reading offers")
            .into_db_result()
    }

    /// Each of `coupon_ids` not yet in an offer wrapped in one of its own whose one grant is the
    /// coupon (the migration's `offer_wrap_coupons`); a grant's own definition is left as it is.
    pub async fn wrap_coupons(conn: &mut PgConn, coupon_ids: &[CouponId]) -> DbResult<()> {
        use diesel::sql_types::{Array, Uuid};

        if coupon_ids.is_empty() {
            return Ok(());
        }
        let ids: Vec<uuid::Uuid> = coupon_ids.iter().map(BaseId::as_uuid).collect();
        diesel::sql_query("SELECT offer_wrap_coupons($1)")
            .bind::<Array<Uuid>, _>(ids)
            .execute(conn)
            .await
            .attach("Error while wrapping coupons in offers")
            .map(|_| ())
            .into_db_result()
    }
}

impl OfferGrantRow {
    pub async fn insert_batch(conn: &mut PgConn, rows: &[OfferGrantRow]) -> DbResult<()> {
        use crate::schema::offer_grant::dsl::offer_grant;

        if rows.is_empty() {
            return Ok(());
        }
        diesel::insert_into(offer_grant)
            .values(rows)
            .execute(conn)
            .await
            .attach("Error while writing an offer's grants")
            .map(|_| ())
            .into_db_result()
    }

    /// The live grants of each of `offers`, in order.
    pub async fn list_live(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        offers: &[OfferId],
    ) -> DbResult<Vec<OfferGrantRow>> {
        use crate::schema::offer_grant::dsl as g;

        if offers.is_empty() {
            return Ok(Vec::new());
        }
        g::offer_grant
            .filter(g::tenant_id.eq(tenant_id_param))
            .filter(g::offer_id.eq_any(offers))
            .filter(g::removed_at.is_null())
            .order((g::offer_id.asc(), g::seq.asc()))
            .select(OfferGrantRow::as_select())
            .get_results(conn)
            .await
            .attach("Error while listing an offer's grants")
            .into_db_result()
    }

    /// Grants by id, those replaced since included.
    pub async fn list_by_ids(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        ids: &[OfferGrantId],
    ) -> DbResult<Vec<OfferGrantRow>> {
        use crate::schema::offer_grant::dsl as g;

        if ids.is_empty() {
            return Ok(Vec::new());
        }
        g::offer_grant
            .filter(g::tenant_id.eq(tenant_id_param))
            .filter(g::id.eq_any(ids))
            .select(OfferGrantRow::as_select())
            .get_results(conn)
            .await
            .attach("Error while reading grants")
            .into_db_result()
    }

    /// The live grants of the offer replaced: applications made before keep theirs.
    pub async fn remove_live(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        offer: OfferId,
        at: NaiveDateTime,
    ) -> DbResult<()> {
        use crate::schema::offer_grant::dsl as g;

        diesel::update(g::offer_grant)
            .filter(g::tenant_id.eq(tenant_id_param))
            .filter(g::offer_id.eq(offer))
            .filter(g::removed_at.is_null())
            .set(g::removed_at.eq(Some(at)))
            .execute(conn)
            .await
            .attach("Error while replacing an offer's grants")
            .map(|_| ())
            .into_db_result()
    }

    /// The grant, if any, whose definition each of `coupons` is.
    pub async fn list_by_coupons(
        conn: &mut PgConn,
        coupons: &[CouponId],
    ) -> DbResult<Vec<OfferGrantRow>> {
        use crate::schema::offer_grant::dsl as g;

        if coupons.is_empty() {
            return Ok(Vec::new());
        }
        g::offer_grant
            .filter(g::coupon_id.eq_any(coupons))
            .select(OfferGrantRow::as_select())
            .get_results(conn)
            .await
            .attach("Error while reading the grants of coupons")
            .into_db_result()
    }
}

impl OfferApplicationRow {
    /// Written once per subscription and offer: `None` when it already was (a replay).
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<Option<OfferApplicationRow>> {
        use crate::schema::offer_application::dsl::offer_application;

        diesel::insert_into(offer_application)
            .values(self)
            .on_conflict_do_nothing()
            .returning(OfferApplicationRow::as_returning())
            .get_result(conn)
            .await
            .optional()
            .attach("Error while recording an offer applied")
            .into_db_result()
    }

    pub async fn find(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        subscription: SubscriptionId,
        offer: OfferId,
    ) -> DbResult<Option<OfferApplicationRow>> {
        use crate::schema::offer_application::dsl as a;

        a::offer_application
            .filter(a::tenant_id.eq(tenant_id_param))
            .filter(a::subscription_id.eq(subscription))
            .filter(a::offer_id.eq(offer))
            .select(OfferApplicationRow::as_select())
            .first(conn)
            .await
            .optional()
            .attach("Error while reading an offer applied")
            .into_db_result()
    }

    pub async fn list_by_subscriptions(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        subscriptions: &[SubscriptionId],
    ) -> DbResult<Vec<OfferApplicationRow>> {
        use crate::schema::offer_application::dsl as a;

        if subscriptions.is_empty() {
            return Ok(Vec::new());
        }
        a::offer_application
            .filter(a::tenant_id.eq(tenant_id_param))
            .filter(a::subscription_id.eq_any(subscriptions))
            .order((a::applied_at.asc(), a::id.asc()))
            .select(OfferApplicationRow::as_select())
            .get_results(conn)
            .await
            .attach("Error while listing the offers applied to subscriptions")
            .into_db_result()
    }

    /// How many times the customer was given the offer, applications withdrawn left out.
    pub async fn count_given(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        customer: CustomerId,
        offer: OfferId,
    ) -> DbResult<i64> {
        use crate::schema::offer_application::dsl as a;

        a::offer_application
            .filter(a::tenant_id.eq(tenant_id_param))
            .filter(a::customer_id.eq(customer))
            .filter(a::offer_id.eq(offer))
            .filter(a::withdrawn_at.is_null())
            .count()
            .get_result(conn)
            .await
            .attach("Error while counting an offer given")
            .into_db_result()
    }

    /// How many times the customer was given, on the plan or product family of `scope`, `offer`
    /// or a plan's start offer; applications withdrawn left out.
    pub async fn count_given_in_scope(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        customer: CustomerId,
        offer: OfferId,
        scope: crate::query::trial_offers::RedemptionScope,
    ) -> DbResult<i64> {
        use crate::query::trial_offers::RedemptionScope;
        use crate::schema::offer_application::dsl as a;
        use crate::schema::plan::dsl as p;
        use crate::schema::plan_version::dsl as pv;
        use crate::schema::subscription::dsl as s;
        use diesel::JoinOnDsl;

        let query = a::offer_application
            .inner_join(s::subscription.on(s::id.eq(a::subscription_id)))
            .inner_join(pv::plan_version.on(pv::id.eq(s::plan_version_id)))
            .inner_join(p::plan.on(p::id.eq(pv::plan_id)))
            .filter(a::tenant_id.eq(tenant_id_param))
            .filter(a::customer_id.eq(customer))
            .filter(a::withdrawn_at.is_null())
            .filter(a::source.eq("PLAN").or(a::offer_id.eq(offer)))
            .count()
            .into_boxed();
        let query = match scope {
            RedemptionScope::Offer(offer) => query.filter(a::offer_id.eq(offer)),
            RedemptionScope::Plan(plan) => query.filter(p::id.eq(plan)),
            RedemptionScope::ProductFamily(family) => query.filter(p::product_family_id.eq(family)),
        };
        query
            .get_result(conn)
            .await
            .attach("Error while counting offers given")
            .into_db_result()
    }

    /// The applications of the subscription its start wrote the grants of, now.
    pub async fn mark_started(
        conn: &mut PgConn,
        ids: &[OfferApplicationId],
        at: NaiveDateTime,
    ) -> DbResult<()> {
        use crate::schema::offer_application::dsl as a;

        if ids.is_empty() {
            return Ok(());
        }
        diesel::update(a::offer_application)
            .filter(a::id.eq_any(ids))
            .filter(a::started_at.is_null())
            .set(a::started_at.eq(Some(at)))
            .execute(conn)
            .await
            .attach("Error while marking offers applied as started")
            .map(|_| ())
            .into_db_result()
    }

    /// How many subscriptions each of `offers` was given to, applications withdrawn aside.
    pub async fn count_by_offers(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        offers: &[OfferId],
    ) -> DbResult<Vec<(OfferId, i64)>> {
        use crate::schema::offer_application::dsl as a;

        if offers.is_empty() {
            return Ok(Vec::new());
        }
        a::offer_application
            .filter(a::tenant_id.eq(tenant_id_param))
            .filter(a::offer_id.eq_any(offers))
            .filter(a::withdrawn_at.is_null())
            .group_by(a::offer_id)
            .select((a::offer_id, diesel::dsl::count_star()))
            .get_results(conn)
            .await
            .attach("Error while counting the subscriptions given offers")
            .into_db_result()
    }

    /// The applications' billing start moved to `on`, as their subscription's trial moved.
    pub async fn move_billing_start(
        conn: &mut PgConn,
        ids: &[OfferApplicationId],
        on: chrono::NaiveDate,
    ) -> DbResult<()> {
        use crate::schema::offer_application::dsl as a;

        if ids.is_empty() {
            return Ok(());
        }
        diesel::update(a::offer_application)
            .filter(a::id.eq_any(ids))
            .set(a::billing_start_on.eq(on))
            .execute(conn)
            .await
            .attach("Error while moving the billing start of offers applied")
            .map(|_| ())
            .into_db_result()
    }

    /// The subscription's applications not withdrawn yet, withdrawn at `at`.
    pub async fn withdraw_for_subscription(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        subscription: SubscriptionId,
        at: NaiveDateTime,
    ) -> DbResult<Vec<OfferApplicationRow>> {
        use crate::schema::offer_application::dsl as a;

        diesel::update(a::offer_application)
            .filter(a::tenant_id.eq(tenant_id_param))
            .filter(a::subscription_id.eq(subscription))
            .filter(a::withdrawn_at.is_null())
            .set(a::withdrawn_at.eq(Some(at)))
            .returning(OfferApplicationRow::as_returning())
            .get_results(conn)
            .await
            .attach("Error while withdrawing the offers of a subscription")
            .into_db_result()
    }

    /// The customers among `customers` who redeemed `coupon`, applications withdrawn left out.
    pub async fn coupon_redeemed_by(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        coupon: CouponId,
        customers: &[CustomerId],
    ) -> DbResult<Vec<CustomerId>> {
        use crate::schema::offer_application::dsl as a;

        if customers.is_empty() {
            return Ok(Vec::new());
        }
        a::offer_application
            .filter(a::tenant_id.eq(tenant_id_param))
            .filter(a::coupon_id.eq(coupon))
            .filter(a::customer_id.eq_any(customers))
            .filter(a::withdrawn_at.is_null())
            .select(a::customer_id)
            .distinct()
            .get_results(conn)
            .await
            .attach("Error while reading the customers a coupon was redeemed by")
            .into_db_result()
    }
}

impl OfferApplicationItemRow {
    /// Each written once: an item already recorded (a replay) is left as it is.
    pub async fn insert_batch(conn: &mut PgConn, rows: &[OfferApplicationItemRow]) -> DbResult<()> {
        use crate::schema::offer_application_item::dsl::offer_application_item;

        if rows.is_empty() {
            return Ok(());
        }
        diesel::insert_into(offer_application_item)
            .values(rows)
            .on_conflict_do_nothing()
            .execute(conn)
            .await
            .attach("Error while recording the rows an offer wrote")
            .map(|_| ())
            .into_db_result()
    }

    pub async fn list_by_applications(
        conn: &mut PgConn,
        applications: &[OfferApplicationId],
    ) -> DbResult<Vec<OfferApplicationItemRow>> {
        use crate::schema::offer_application_item::dsl as i;

        if applications.is_empty() {
            return Ok(Vec::new());
        }
        i::offer_application_item
            .filter(i::application_id.eq_any(applications))
            .order((i::application_id.asc(), i::created_at.asc()))
            .select(OfferApplicationItemRow::as_select())
            .get_results(conn)
            .await
            .attach("Error while listing the rows offers wrote")
            .into_db_result()
    }

    /// The open item's window end set again, as its window moved.
    pub async fn set_ends_at(
        conn: &mut PgConn,
        kind_param: &str,
        target: uuid::Uuid,
        ends_at_param: Option<NaiveDateTime>,
    ) -> DbResult<()> {
        use crate::schema::offer_application_item::dsl as i;

        diesel::update(i::offer_application_item)
            .filter(i::kind.eq(kind_param))
            .filter(i::target_id.eq(target))
            .filter(i::ended_at.is_null())
            .set(i::ends_at.eq(ends_at_param))
            .execute(conn)
            .await
            .attach("Error while dating the end of a row an offer wrote")
            .map(|_| ())
            .into_db_result()
    }

    /// The items of `applications` not ended yet, ended at `at`.
    pub async fn end_for_applications(
        conn: &mut PgConn,
        applications: &[OfferApplicationId],
        at: NaiveDateTime,
    ) -> DbResult<Vec<OfferApplicationItemRow>> {
        use crate::schema::offer_application_item::dsl as i;

        if applications.is_empty() {
            return Ok(Vec::new());
        }
        diesel::update(i::offer_application_item)
            .filter(i::application_id.eq_any(applications))
            .filter(i::ended_at.is_null())
            .set(i::ended_at.eq(Some(at)))
            .returning(OfferApplicationItemRow::as_returning())
            .get_results(conn)
            .await
            .attach("Error while ending the rows offers wrote")
            .into_db_result()
    }
}

/// The open items of started applications whose grant's window is over at `now` (`ends_at`, kept
/// on the item), ended at its end: a grant's items at once, at most `limit` grants, earliest
/// first. A pass replayed or run twice claims nothing more.
const CLAIM_WINDOWS_ENDED: &str = "
WITH due AS (
    SELECT i.application_id, i.grant_id, min(i.ends_at) AS ends_at
    FROM offer_application_item i
    JOIN offer_application a ON a.id = i.application_id
    WHERE i.ended_at IS NULL
      AND i.ends_at IS NOT NULL
      AND i.ends_at <= $1
      AND a.withdrawn_at IS NULL
      AND a.started_at IS NOT NULL
    GROUP BY i.application_id, i.grant_id
    ORDER BY min(i.ends_at), i.application_id, i.grant_id
    LIMIT $2
)
UPDATE offer_application_item i SET ended_at = due.ends_at
FROM due
WHERE i.application_id = due.application_id AND i.grant_id = due.grant_id
  AND i.ended_at IS NULL
RETURNING i.*";

impl OfferApplicationItemRow {
    pub async fn claim_windows_ended(
        conn: &mut PgConn,
        now: NaiveDateTime,
        limit: i64,
    ) -> DbResult<Vec<OfferApplicationItemRow>> {
        use diesel::sql_types::{BigInt, Timestamp};

        diesel::sql_query(CLAIM_WINDOWS_ENDED)
            .bind::<Timestamp, _>(now)
            .bind::<BigInt, _>(limit)
            .load(conn)
            .await
            .attach("Error while claiming the grants whose window ended")
            .into_db_result()
    }
}

impl OfferApplicationRow {
    pub async fn list_by_ids(
        conn: &mut PgConn,
        ids: &[OfferApplicationId],
    ) -> DbResult<Vec<OfferApplicationRow>> {
        use crate::schema::offer_application::dsl as a;

        if ids.is_empty() {
            return Ok(Vec::new());
        }
        a::offer_application
            .filter(a::id.eq_any(ids))
            .select(OfferApplicationRow::as_select())
            .get_results(conn)
            .await
            .attach("Error while listing offers applied")
            .into_db_result()
    }
}

/// The retention offers giving each of `offers`, by offer.
pub async fn retention_offers_of(
    conn: &mut PgConn,
    tenant_id_param: TenantId,
    offers: &[OfferId],
) -> DbResult<Vec<(OfferId, uuid::Uuid)>> {
    use crate::schema::retention_offer::dsl as r;

    if offers.is_empty() {
        return Ok(Vec::new());
    }
    r::retention_offer
        .filter(r::tenant_id.eq(tenant_id_param))
        .filter(r::offer_id.eq_any(offers))
        .filter(r::archived_at.is_null())
        .select((r::offer_id.assume_not_null(), r::id))
        .get_results(conn)
        .await
        .attach("Error while listing the retention offers of offers")
        .into_db_result()
}

impl OfferApplicationItemRow {
    /// The items of `targets` not ended yet, ended at `at`.
    pub async fn end_targets(
        conn: &mut PgConn,
        targets: &[uuid::Uuid],
        at: NaiveDateTime,
    ) -> DbResult<Vec<OfferApplicationItemRow>> {
        use crate::schema::offer_application_item::dsl as i;

        if targets.is_empty() {
            return Ok(Vec::new());
        }
        diesel::update(i::offer_application_item)
            .filter(i::target_id.eq_any(targets))
            .filter(i::ended_at.is_null())
            .set(i::ended_at.eq(Some(at)))
            .returning(OfferApplicationItemRow::as_returning())
            .get_results(conn)
            .await
            .attach("Error while ending the rows offers wrote")
            .into_db_result()
    }
}

impl CouponRow {
    /// The coupons naming each of `offers` as theirs (codes of an offer).
    pub async fn list_by_offers(
        conn: &mut PgConn,
        tenant_id_param: TenantId,
        offers: &[OfferId],
    ) -> DbResult<Vec<CouponRow>> {
        use crate::schema::coupon::dsl as c;

        if offers.is_empty() {
            return Ok(Vec::new());
        }
        c::coupon
            .filter(c::tenant_id.eq(tenant_id_param))
            .filter(c::offer_id.eq_any(offers))
            .filter(c::archived_at.is_null())
            .select(CouponRow::as_select())
            .get_results(conn)
            .await
            .attach("Error while listing the coupons of offers")
            .into_db_result()
    }

    /// Whether the coupon is an offer's grant definition: no code to read or redeem.
    pub async fn is_grant_definition(conn: &mut PgConn, id: CouponId) -> DbResult<bool> {
        use crate::schema::coupon::dsl as c;
        use crate::schema::offer_grant::dsl as g;

        diesel::select(diesel::dsl::exists(
            g::offer_grant
                .inner_join(c::coupon)
                .filter(g::coupon_id.eq(id))
                .filter(
                    c::offer_id
                        .is_null()
                        .or(c::offer_id.ne(g::offer_id.nullable())),
                ),
        ))
        .get_result(conn)
        .await
        .attach("Error while reading whether a coupon is a grant's")
        .into_db_result()
    }
}
