use std::collections::HashMap;

use chrono::NaiveDateTime;
use common_domain::ids::{
    BaseId, CheckoutSessionId, CustomerId, PlanId, PurchaseLinkId, PurchaseLinkSessionId,
    SubscriptionId, TenantId,
};
use diesel_models::checkout_sessions::CheckoutSessionRow;
use diesel_models::customers::CustomerRow;
use diesel_models::purchase_links::{
    PurchaseBindingRow, PurchaseLinkRow, PurchaseLinkRowNew, PurchaseLinkRowUpdate,
    PurchaseLinkSessionRestart, PurchaseLinkSessionRow, PurchaseLinkSessionRowNew,
    PurchasePowSpentRowNew,
};
use error_stack::Report;
use scoped_futures::ScopedFutureExt;

use crate::domain::enums::{PlanStatusEnum, PlanTypeEnum};
use crate::domain::purchase_links::{
    InstanceId, LivePurchaseLink, PURCHASE_LINK_TOKEN_LENGTH, PurchaseAttemptRestart, PurchaseLink,
    PurchaseLinkNew, PurchaseLinkSession, PurchaseLinkSessionNew, PurchaseLinkUpdate,
    PurchaseLinkWithStats, PurchaseRecordConflict, generate_purchase_link_token,
};
pub use crate::domain::purchase_links::{ListedPurchaseLink, PurchaseBinding};
use crate::domain::{Plan, PlanVersion, PlanVersionFilter};
use crate::errors::StoreError;
use crate::repositories::plans::PlansInterface;
use crate::store::PgConn;
use crate::{Store, StoreResult};

const MAX_NAME_LENGTH: usize = 200;

#[async_trait::async_trait]
pub trait PurchaseLinksInterface {
    async fn create_purchase_link(
        &self,
        link: PurchaseLinkNew,
    ) -> StoreResult<PurchaseLinkWithStats>;

    async fn list_purchase_links(
        &self,
        tenant_id: TenantId,
        plan_id: Option<PlanId>,
        include_archived: bool,
    ) -> StoreResult<Vec<PurchaseLinkWithStats>>;

    async fn get_purchase_link(
        &self,
        tenant_id: TenantId,
        id: PurchaseLinkId,
    ) -> StoreResult<PurchaseLinkWithStats>;

    async fn update_purchase_link(
        &self,
        tenant_id: TenantId,
        id: PurchaseLinkId,
        update: PurchaseLinkUpdate,
    ) -> StoreResult<PurchaseLinkWithStats>;

    /// Replaces the token: the previous URL stops working.
    async fn regenerate_purchase_link_token(
        &self,
        tenant_id: TenantId,
        id: PurchaseLinkId,
    ) -> StoreResult<PurchaseLinkWithStats>;

    async fn archive_purchase_link(
        &self,
        tenant_id: TenantId,
        id: PurchaseLinkId,
    ) -> StoreResult<PurchaseLinkWithStats>;

    /// The link behind a public token, when it can be bought from (not archived, plan
    /// active with a published version). `None` otherwise, whatever the reason.
    async fn find_live_purchase_link(&self, token: &str) -> StoreResult<Option<LivePurchaseLink>>;

    async fn find_purchase_attempt(
        &self,
        tenant_id: TenantId,
        purchase_link_id: PurchaseLinkId,
        attempt_id: &str,
    ) -> StoreResult<Option<PurchaseLinkSession>>;

    /// Records a proof-of-work challenge as used; `false` when it already was (a replay).
    async fn spend_pow_challenge(
        &self,
        challenge: &str,
        expires_at: NaiveDateTime,
    ) -> StoreResult<bool>;

    async fn purge_spent_pow_challenges(&self) -> StoreResult<usize>;

    /// Declined checkout charges of a purchase customer, across its restarts.
    async fn count_purchase_declines(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
    ) -> StoreResult<i64>;

    async fn insert_purchase_link_session(
        &self,
        session: PurchaseLinkSessionNew,
    ) -> StoreResult<Result<PurchaseLinkSession, PurchaseRecordConflict>>;

    /// Points the attempt at a new checkout, unless it moved since it was read (its checkout
    /// is no longer `previous_checkout`, or the reused customer was cleaned up meanwhile).
    async fn restart_purchase_attempt(
        &self,
        tenant_id: TenantId,
        id: PurchaseLinkSessionId,
        previous_checkout: Option<CheckoutSessionId>,
        reuses_customer: bool,
        restart: PurchaseAttemptRestart,
    ) -> StoreResult<Result<(), PurchaseRecordConflict>>;

    /// False only for checkout sessions started from a link that refuses coupon codes.
    async fn checkout_accepts_coupon_codes(
        &self,
        tenant_id: TenantId,
        checkout_session_id: CheckoutSessionId,
    ) -> StoreResult<bool>;

    /// Archives the customers of purchases abandoned before `older_than`. Returns how many.
    async fn cleanup_abandoned_purchases(
        &self,
        older_than: NaiveDateTime,
        batch_size: i64,
    ) -> StoreResult<usize>;

    async fn list_listed_purchase_links(
        &self,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<ListedPurchaseLink>>;

    /// Binding of a completed purchase (checkout_session.subscription_id set) started by licensed software.
    async fn find_purchase_binding_by_subscription(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
    ) -> StoreResult<Option<PurchaseBinding>>;

    /// Every completed purchase started by an installation id, newest first. Ids are not
    /// secret: callers keep the one whose key proves the claim.
    async fn find_purchase_binding_by_instance(
        &self,
        tenant_id: TenantId,
        instance_id: &InstanceId,
    ) -> StoreResult<Vec<PurchaseBinding>>;
}

#[async_trait::async_trait]
impl PurchaseLinksInterface for Store {
    async fn create_purchase_link(
        &self,
        link: PurchaseLinkNew,
    ) -> StoreResult<PurchaseLinkWithStats> {
        let name = validate_name(&link.name)?;
        validate_redirect_url("success_url", link.success_url.as_deref())?;
        validate_redirect_url("cancel_url", link.cancel_url.as_deref())?;

        let plan = self
            .get_plan(link.plan_id, link.tenant_id, PlanVersionFilter::Active)
            .await
            .map_err(|_| {
                Report::new(StoreError::InvalidArgument(
                    "The plan has no published version".to_string(),
                ))
            })?;
        ensure_purchasable(&plan.plan, plan.version.as_ref())?;
        ensure_listable(link.listed, plan.plan.issues_license)?;

        let mut conn = self.get_conn().await?;
        let row = PurchaseLinkRowNew {
            id: PurchaseLinkId::new(),
            tenant_id: link.tenant_id,
            plan_id: link.plan_id,
            name,
            token: generate_purchase_link_token(),
            allow_coupon_codes: link.allow_coupon_codes,
            listed: link.listed,
            success_url: link.success_url,
            cancel_url: link.cancel_url,
            require_billing_address: link.require_billing_address,
            tax_id_collection: link.tax_id_collection.into(),
            only_buyer_type: link.allowed_buyer_types.to_row(),
        }
        .insert(&mut conn)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        Ok(PurchaseLinkWithStats {
            link: row.into(),
            plan_name: plan.plan.name,
            started_count: 0,
            completed_count: 0,
        })
    }

    async fn list_purchase_links(
        &self,
        tenant_id: TenantId,
        plan_id: Option<PlanId>,
        include_archived: bool,
    ) -> StoreResult<Vec<PurchaseLinkWithStats>> {
        let mut conn = self.get_conn().await?;
        let rows = PurchaseLinkRow::list(&mut conn, tenant_id, plan_id, include_archived)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        with_stats(&mut conn, tenant_id, rows).await
    }

    async fn get_purchase_link(
        &self,
        tenant_id: TenantId,
        id: PurchaseLinkId,
    ) -> StoreResult<PurchaseLinkWithStats> {
        let mut conn = self.get_conn().await?;
        let row = PurchaseLinkRow::find_by_id(&mut conn, tenant_id, id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        with_plan_and_stats(self, &mut conn, row).await
    }

    async fn update_purchase_link(
        &self,
        tenant_id: TenantId,
        id: PurchaseLinkId,
        update: PurchaseLinkUpdate,
    ) -> StoreResult<PurchaseLinkWithStats> {
        let name = validate_name(&update.name)?;
        validate_redirect_url("success_url", update.success_url.as_deref())?;
        validate_redirect_url("cancel_url", update.cancel_url.as_deref())?;

        let mut conn = self.get_conn().await?;
        if update.listed {
            let current = PurchaseLinkRow::find_by_id(&mut conn, tenant_id, id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
            let plan = self.get_plan_overview(current.plan_id, tenant_id).await?;
            ensure_listable(true, plan.issues_license)?;
        }
        let row = PurchaseLinkRow::update(
            &mut conn,
            tenant_id,
            id,
            PurchaseLinkRowUpdate {
                name,
                allow_coupon_codes: update.allow_coupon_codes,
                listed: update.listed,
                success_url: update.success_url,
                cancel_url: update.cancel_url,
                require_billing_address: update.require_billing_address,
                tax_id_collection: update.tax_id_collection.into(),
                only_buyer_type: update.allowed_buyer_types.to_row(),
            },
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .ok_or_else(not_found_or_archived)?;
        with_plan_and_stats(self, &mut conn, row).await
    }

    async fn regenerate_purchase_link_token(
        &self,
        tenant_id: TenantId,
        id: PurchaseLinkId,
    ) -> StoreResult<PurchaseLinkWithStats> {
        let mut conn = self.get_conn().await?;
        let row =
            PurchaseLinkRow::set_token(&mut conn, tenant_id, id, &generate_purchase_link_token())
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .ok_or_else(not_found_or_archived)?;
        with_plan_and_stats(self, &mut conn, row).await
    }

    async fn archive_purchase_link(
        &self,
        tenant_id: TenantId,
        id: PurchaseLinkId,
    ) -> StoreResult<PurchaseLinkWithStats> {
        let mut conn = self.get_conn().await?;
        let row = PurchaseLinkRow::archive(&mut conn, tenant_id, id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        with_plan_and_stats(self, &mut conn, row).await
    }

    async fn find_live_purchase_link(&self, token: &str) -> StoreResult<Option<LivePurchaseLink>> {
        if token.len() != PURCHASE_LINK_TOKEN_LENGTH
            || !token.chars().all(|c| c.is_ascii_alphanumeric())
        {
            return Ok(None);
        }

        let mut conn = self.get_conn().await?;
        let Some(row) = PurchaseLinkRow::find_unarchived_by_token(&mut conn, token)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
        else {
            return Ok(None);
        };
        drop(conn);

        let plan = match self
            .get_plan(row.plan_id, row.tenant_id, PlanVersionFilter::Active)
            .await
        {
            Ok(plan) => plan,
            Err(e) => {
                log::debug!("Purchase link {} has no buyable plan: {e:?}", row.id);
                return Ok(None);
            }
        };
        let Some(version) = plan.version.clone() else {
            return Ok(None);
        };
        if ensure_purchasable(&plan.plan, Some(&version)).is_err() {
            return Ok(None);
        }

        Ok(Some(LivePurchaseLink {
            link: row.into(),
            plan: plan.plan,
            version,
        }))
    }

    async fn find_purchase_attempt(
        &self,
        tenant_id: TenantId,
        purchase_link_id: PurchaseLinkId,
        attempt_id: &str,
    ) -> StoreResult<Option<PurchaseLinkSession>> {
        let mut conn = self.get_conn().await?;
        PurchaseLinkSessionRow::find_by_attempt(&mut conn, tenant_id, purchase_link_id, attempt_id)
            .await
            .map(|row| row.map(Into::into))
            .map_err(Into::<Report<StoreError>>::into)
    }

    async fn spend_pow_challenge(
        &self,
        challenge: &str,
        expires_at: NaiveDateTime,
    ) -> StoreResult<bool> {
        let mut conn = self.get_conn().await?;
        let spent = PurchasePowSpentRowNew {
            challenge: challenge.to_string(),
            expires_at,
        }
        .insert(&mut conn)
        .await
        .map_err(Into::<Report<StoreError>>::into);
        conflict_or(spent).map(|r| r.is_ok())
    }

    async fn purge_spent_pow_challenges(&self) -> StoreResult<usize> {
        let mut conn = self.get_conn().await?;
        PurchasePowSpentRowNew::delete_expired(&mut conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)
    }

    async fn count_purchase_declines(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
    ) -> StoreResult<i64> {
        let mut conn = self.get_conn().await?;
        PurchaseLinkSessionRow::count_declines_for_customer(&mut conn, tenant_id, customer_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
    }

    async fn insert_purchase_link_session(
        &self,
        session: PurchaseLinkSessionNew,
    ) -> StoreResult<Result<PurchaseLinkSession, PurchaseRecordConflict>> {
        let mut conn = self.get_conn().await?;
        let inserted = PurchaseLinkSessionRowNew {
            id: PurchaseLinkSessionId::new(),
            tenant_id: session.tenant_id,
            purchase_link_id: session.purchase_link_id,
            customer_id: session.customer_id,
            checkout_session_id: Some(session.checkout_session_id),
            attempt_id: session.attempt_id,
            license_instance_id: session.license_instance_id.map(InstanceId::into_string),
            license_instance_public_key: session.license_instance_public_key,
        }
        .insert(&mut conn)
        .await
        .map_err(Into::<Report<StoreError>>::into);
        conflict_or(inserted.map(Into::into))
    }

    async fn restart_purchase_attempt(
        &self,
        tenant_id: TenantId,
        id: PurchaseLinkSessionId,
        previous_checkout: Option<CheckoutSessionId>,
        reuses_customer: bool,
        restart: PurchaseAttemptRestart,
    ) -> StoreResult<Result<(), PurchaseRecordConflict>> {
        let mut conn = self.get_conn().await?;
        let updated = PurchaseLinkSessionRow::restart(
            &mut conn,
            tenant_id,
            id,
            previous_checkout,
            reuses_customer,
            PurchaseLinkSessionRestart {
                customer_id: restart.customer_id,
                checkout_session_id: restart.checkout_session_id,
                license_instance_id: restart.license_instance_id.map(InstanceId::into_string),
                license_instance_public_key: restart.license_instance_public_key,
            },
        )
        .await
        .map_err(Into::<Report<StoreError>>::into);
        match conflict_or(updated)? {
            Ok(1) => Ok(Ok(())),
            _ => Ok(Err(PurchaseRecordConflict)),
        }
    }

    async fn checkout_accepts_coupon_codes(
        &self,
        tenant_id: TenantId,
        checkout_session_id: CheckoutSessionId,
    ) -> StoreResult<bool> {
        let mut conn = self.get_conn().await?;
        checkout_accepts_coupon_codes_tx(&mut conn, tenant_id, checkout_session_id).await
    }

    async fn cleanup_abandoned_purchases(
        &self,
        older_than: NaiveDateTime,
        batch_size: i64,
    ) -> StoreResult<usize> {
        let candidates = {
            let mut conn = self.get_conn().await?;
            PurchaseLinkSessionRow::list_abandoned(&mut conn, older_than, batch_size)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
        };

        let mut cleaned = 0;
        for candidate in candidates {
            let result = self
                .transaction(|conn| {
                    let candidate = candidate.clone();
                    async move {
                        let lock =
                            CustomerRow::lock(conn, candidate.tenant_id, candidate.customer_id)
                                .await
                                .map_err(Into::<Report<StoreError>>::into)?;
                        let Some(candidate) = PurchaseLinkSessionRow::lock_abandoned(
                            conn, &lock, &candidate, older_than,
                        )
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                        else {
                            return Ok(false);
                        };
                        if let Some(checkout_session_id) = candidate.checkout_session_id {
                            CheckoutSessionRow::mark_cancelled(
                                conn,
                                candidate.tenant_id,
                                checkout_session_id,
                            )
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                        }
                        CustomerRow::archive(conn, candidate.customer_id, candidate.tenant_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                        PurchaseLinkSessionRow::mark_cleaned_up(conn, &[candidate.id])
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                        Ok(true)
                    }
                    .scope_boxed()
                })
                .await;

            match result {
                Ok(true) => cleaned += 1,
                Ok(false) => {}
                Err(e) => log::warn!(
                    "Could not clean up abandoned purchase {}: {e:?}",
                    candidate.id
                ),
            }
        }

        Ok(cleaned)
    }

    async fn list_listed_purchase_links(
        &self,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<ListedPurchaseLink>> {
        let mut conn = self.get_conn().await?;
        let rows = PurchaseLinkRow::list_listed(&mut conn, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(rows
            .into_iter()
            .map(|row| ListedPurchaseLink {
                id: row.id,
                name: row.name,
                plan_name: row.plan_name,
                plan_description: row.plan_description,
                token: row.token,
            })
            .collect())
    }

    async fn find_purchase_binding_by_subscription(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
    ) -> StoreResult<Option<PurchaseBinding>> {
        let mut conn = self.get_conn().await?;
        PurchaseLinkSessionRow::find_binding_by_subscription(&mut conn, tenant_id, subscription_id)
            .await
            .map(|row| row.map(binding_from_row))
            .map_err(Into::<Report<StoreError>>::into)
    }

    async fn find_purchase_binding_by_instance(
        &self,
        tenant_id: TenantId,
        instance_id: &InstanceId,
    ) -> StoreResult<Vec<PurchaseBinding>> {
        let mut conn = self.get_conn().await?;
        PurchaseLinkSessionRow::find_binding_by_instance(&mut conn, tenant_id, instance_id.as_str())
            .await
            .map(|rows| rows.into_iter().map(binding_from_row).collect())
            .map_err(Into::<Report<StoreError>>::into)
    }
}

pub(crate) async fn checkout_accepts_coupon_codes_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    checkout_session_id: CheckoutSessionId,
) -> StoreResult<bool> {
    PurchaseLinkRow::coupon_codes_allowed_for_checkout(conn, tenant_id, checkout_session_id)
        .await
        .map(|allowed| allowed.unwrap_or(true))
        .map_err(Into::<Report<StoreError>>::into)
}

/// Only Active, non-Custom plans with a published version can be bought from a link.
pub fn ensure_purchasable(plan: &Plan, version: Option<&PlanVersion>) -> Result<(), StoreError> {
    if plan.status != PlanStatusEnum::Active {
        return Err(StoreError::InvalidArgument(
            "Only active plans can be sold through a purchase link".to_string(),
        ));
    }
    if plan.plan_type == PlanTypeEnum::Custom {
        return Err(StoreError::InvalidArgument(
            "Custom plans can't be sold through a purchase link".to_string(),
        ));
    }
    match version {
        Some(v) if !v.is_draft_version && plan.active_version_id == Some(v.id) => Ok(()),
        _ => Err(StoreError::InvalidArgument(
            "The plan has no published version".to_string(),
        )),
    }
}

/// Listed links are offered to licensed software, so they must sell a license.
fn ensure_listable(listed: bool, issues_license: bool) -> Result<(), StoreError> {
    if listed && !issues_license {
        return Err(StoreError::InvalidArgument(
            "Only links to plans that issue a license can be listed".to_string(),
        ));
    }
    Ok(())
}

/// Redirect targets are reflected to the buyer's browser: absolute http(s) URLs only.
pub fn validate_redirect_url(field: &str, value: Option<&str>) -> Result<(), StoreError> {
    let Some(value) = value else {
        return Ok(());
    };
    let is_http = url::Url::parse(value)
        .map(|u| matches!(u.scheme(), "http" | "https") && u.host().is_some())
        .unwrap_or(false);
    if is_http {
        Ok(())
    } else {
        Err(StoreError::InvalidArgument(format!(
            "{field} must be an absolute http(s) URL"
        )))
    }
}

fn validate_name(name: &str) -> Result<String, StoreError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME_LENGTH {
        return Err(StoreError::InvalidArgument(format!(
            "name must be between 1 and {MAX_NAME_LENGTH} characters"
        )));
    }
    Ok(name.to_string())
}

fn not_found_or_archived() -> Report<StoreError> {
    Report::new(StoreError::ValueNotFound(
        "Purchase link not found or archived".to_string(),
    ))
}

fn binding_from_row(row: PurchaseBindingRow) -> PurchaseBinding {
    PurchaseBinding {
        subscription_id: row.subscription_id,
        instance_id: InstanceId::stored(row.instance_id),
        instance_public_key: row.instance_public_key,
    }
}

/// Unique violations (a replayed proof of work, a concurrent attempt) are expected outcomes.
fn conflict_or<T>(result: StoreResult<T>) -> StoreResult<Result<T, PurchaseRecordConflict>> {
    match result {
        Ok(value) => Ok(Ok(value)),
        Err(e) if matches!(e.current_context(), StoreError::DuplicateValue { .. }) => {
            Ok(Err(PurchaseRecordConflict))
        }
        Err(e) => Err(e),
    }
}

async fn with_plan_and_stats(
    store: &Store,
    conn: &mut PgConn,
    row: PurchaseLinkRow,
) -> StoreResult<PurchaseLinkWithStats> {
    let plan = store.get_plan_overview(row.plan_id, row.tenant_id).await?;
    let tenant_id = row.tenant_id;
    with_stats(conn, tenant_id, vec![(row, plan.name)])
        .await?
        .pop()
        .ok_or_else(not_found_or_archived)
}

async fn with_stats(
    conn: &mut PgConn,
    tenant_id: TenantId,
    rows: Vec<(PurchaseLinkRow, String)>,
) -> StoreResult<Vec<PurchaseLinkWithStats>> {
    let ids: Vec<PurchaseLinkId> = rows.iter().map(|(row, _)| row.id).collect();
    let stats: HashMap<PurchaseLinkId, (i64, i64)> =
        PurchaseLinkSessionRow::stats(conn, tenant_id, &ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .map(|s| (s.purchase_link_id, (s.started, s.completed)))
            .collect();

    Ok(rows
        .into_iter()
        .map(|(row, plan_name)| {
            let (started, completed) = stats.get(&row.id).copied().unwrap_or((0, 0));
            PurchaseLinkWithStats {
                link: PurchaseLink::from(row),
                plan_name,
                started_count: u32::try_from(started).unwrap_or(u32::MAX),
                completed_count: u32::try_from(completed).unwrap_or(u32::MAX),
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirect_urls_must_be_absolute_http() {
        assert!(validate_redirect_url("u", Some("https://example.com/ok")).is_ok());
        assert!(validate_redirect_url("u", None).is_ok());
        assert!(validate_redirect_url("u", Some("javascript:alert(1)")).is_err());
        assert!(validate_redirect_url("u", Some("/relative")).is_err());
    }

    #[test]
    fn names_are_trimmed_and_bounded() {
        assert_eq!(validate_name("  Pro  ").unwrap(), "Pro");
        assert!(validate_name("   ").is_err());
        assert!(validate_name(&"x".repeat(MAX_NAME_LENGTH + 1)).is_err());
    }
}
