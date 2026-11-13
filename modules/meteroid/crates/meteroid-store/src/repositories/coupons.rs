use crate::domain::coupons::{Coupon, CouponFilter, CouponNew, CouponPatch, CouponStatusPatch};
use crate::domain::entity_activity::Actor;
use crate::domain::outbox_event::OutboxEvent;
use crate::domain::{AppliedCouponForDisplay, PaginatedVec, PaginationRequest};
use crate::errors::StoreError;
use crate::store::PgConn;
use crate::{Store, StoreResult};
use common_domain::ids::{CouponId, TenantId};
use diesel_models::applied_coupons::AppliedCouponForDisplayRow;
use diesel_models::coupon_plans::{CouponPlanRow, CouponPlanRowNew};
use diesel_models::coupons::{CouponRow, CouponRowNew, CouponRowPatch, CouponStatusRowPatch};
use diesel_models::query::coupons::CodeMatch;
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
    ) -> StoreResult<PaginatedVec<Coupon>>;
    #[delegated]
    async fn get_coupon_by_id(&self, tenant_id: TenantId, id: CouponId) -> StoreResult<Coupon>;

    async fn create_coupon(&self, actor: Actor, coupon: CouponNew) -> StoreResult<Coupon>;
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
    ) -> StoreResult<PaginatedVec<Coupon>> {
        let mut conn = self.get_conn().await?;

        let coupons = CouponRow::list_by_tenant_id(
            &mut conn,
            tenant_id,
            pagination.into(),
            search,
            filter.into(),
            order_by.as_deref(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        // Batch fetch plan_ids for all coupons
        let coupon_ids: Vec<CouponId> = coupons.items.iter().map(|c| c.id).collect();
        let plan_ids_map = CouponPlanRow::list_by_coupon_ids(&mut conn, &coupon_ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let items: Vec<Coupon> = coupons
            .items
            .into_iter()
            .map(|row| {
                let id = row.id;
                let mut coupon: Coupon = row.try_into()?;
                coupon.plan_ids = plan_ids_map.get(&id).cloned().unwrap_or_default();
                Ok(coupon)
            })
            .collect::<Result<Vec<_>, Report<StoreError>>>()?;

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

        let plan_ids = CouponPlanRow::list_by_coupon_id(conn, id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let mut coupon: Coupon = row.try_into()?;
        coupon.plan_ids = plan_ids;
        Ok(coupon)
    }

    async fn create_coupon(&self, actor: Actor, mut coupon: CouponNew) -> StoreResult<Coupon> {
        let mut conn = self.get_conn().await?;

        coupon.discount.validate()?;
        if coupon.discount.is_credit() && coupon.recurring_value.is_some() {
            return Err(Report::new(StoreError::InvalidArgument(
                "a promotional credit coupon must not have a recurring value".to_owned(),
            )));
        }
        coupon.code = normalize_coupon_code(&coupon)?;
        let is_credit = coupon.discount.is_credit();
        let tenant_id = coupon.tenant_id;
        let plan_ids = coupon.plan_ids.clone();
        let coupon_row: CouponRowNew = coupon.try_into()?;
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

                    let mut coupon: Coupon = row.try_into()?;
                    coupon.plan_ids = plan_ids;

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

    async fn update_coupon(&self, actor: Actor, coupon: CouponPatch) -> StoreResult<Coupon> {
        let tenant_id = coupon.tenant_id;
        let coupon_id = coupon.id;
        let plan_ids = coupon.plan_ids.clone();
        let new_discount = coupon.discount.clone();
        if let Some(discount) = &new_discount {
            discount.validate()?;
        }
        let coupon_row: CouponRowPatch = coupon.try_into()?;

        self.transaction_bumping_tenant(tenant_id, |conn| {
            let actor = &actor;
            async move {
                if let Some(discount) = &new_discount {
                    let current: Coupon = CouponRow::get_by_id(conn, tenant_id, coupon_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                        .try_into()?;
                    // Past redemptions were granted credit or discounted, never both.
                    if current.discount.is_credit() != discount.is_credit() {
                        return Err(Report::new(StoreError::InvalidArgument(
                            "a coupon cannot switch between promotional credit and an invoice discount"
                                .to_owned(),
                        )));
                    }
                }
                let row = coupon_row
                    .patch(conn)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                let final_plan_ids = if let Some(new_plan_ids) = plan_ids {
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

                let mut coupon: Coupon = row.try_into()?;
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
        let coupon_id = coupon.id;
        let is_archiving = coupon.archived_at.as_ref().is_some_and(|v| v.is_some());
        let coupon_row: CouponStatusRowPatch = coupon.into();

        self.transaction_with(&mut conn, |conn| {
            let actor = &actor;
            async move {
                let row = coupon_row
                    .patch(conn)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                let plan_ids = CouponPlanRow::list_by_coupon_id(conn, coupon_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                let mut coupon: Coupon = row.try_into()?;
                coupon.plan_ids = plan_ids;

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

        // Batch fetch plan_ids for all coupons
        let coupon_ids: Vec<CouponId> = rows.iter().map(|c| c.id).collect();
        let plan_ids_map = CouponPlanRow::list_by_coupon_ids(conn, &coupon_ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        rows.into_iter()
            .map(|row| {
                let id = row.id;
                let mut coupon: Coupon = row.try_into()?;
                coupon.plan_ids = plan_ids_map.get(&id).cloned().unwrap_or_default();
                Ok(coupon)
            })
            .collect()
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use common_domain::ids::BaseId;

    fn row(code: &str) -> CouponRow {
        let now = common_domain::clock::now().naive_utc();
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
