//! Starts the customers of a product line with no plan there on its free sign-up plan.
use std::collections::BTreeSet;

use crate::StoreResult;
use crate::domain::Address;
use crate::domain::eligibility::{self, CustomerFacts};
use crate::domain::outbox_event::SubscriptionOrigin;
use crate::errors::StoreError;
use crate::repositories::credit::customer_credit::lock_customer_tx;
use crate::repositories::customers::CustomersInterfaceAuto;
use crate::services::ServicesEdge;
use crate::services::signup::SignupPlan;
use common_domain::actor::Actor;
use common_domain::ids::{
    BaseId, BatchJobId, CustomerId, PlanVersionId, ProductFamilyId, SubscriptionId, TenantId,
};
use diesel_models::batch_jobs::BatchJobEntityRowNew;
use diesel_models::query::product_family_journeys::{
    count_planless, count_unfinished, is_planless, planless_customer_facts, planless_customer_ids,
};
use error_stack::Report;
use scoped_futures::ScopedFutureExt;

/// Who a backfill would start on the sign-up plan, and who it leaves off, as counts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignupBackfillPreview {
    pub plan_name: String,
    /// Started on the plan.
    pub customers: u32,
    /// A subscription of theirs in the product line ended: started only when asked for.
    pub churned: u32,
    /// Left off: they pay in a currency the plan is not sold in.
    pub not_sold: u32,
    /// The plan has an eligibility rule: the backfill skips the customers it leaves off.
    pub eligibility_rule: bool,
    /// Left off by the plan's eligibility rule.
    pub not_eligible: u32,
    /// Left off: their start here is unfinished (pending activation, or a trial whose checkout
    /// never settled), so they hold a plan already.
    pub unfinished: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackfillOutcome {
    Subscribed(SubscriptionId),
    /// Archived, or on a plan of the product line, since the preview.
    Skipped,
    /// Left off by the plan's eligibility rule.
    NotEligible,
}

impl ServicesEdge {
    async fn free_signup_plan(
        &self,
        tenant_id: TenantId,
        product_family_id: ProductFamilyId,
    ) -> StoreResult<SignupPlan> {
        let plan = self.signup_plan(tenant_id, Some(product_family_id)).await?;
        if !plan.free() {
            return Err(Report::new(StoreError::InvalidArgument(
                "Only a free sign-up plan can start existing customers: this one charges, grants \
                 credit, reads usage or has a minimum commitment"
                    .to_owned(),
            )));
        }
        Ok(plan)
    }

    /// How many customers of `product_family_id` with no plan there a backfill would start on its
    /// free sign-up plan; with `include_churned`, those whose subscription there ended too.
    pub async fn signup_backfill_preview(
        &self,
        tenant_id: TenantId,
        product_family_id: ProductFamilyId,
        include_churned: bool,
    ) -> StoreResult<SignupBackfillPreview> {
        let plan = self.free_signup_plan(tenant_id, product_family_id).await?;
        let mut conn = self.store.get_conn().await?;
        let counts = count_planless(&mut conn, tenant_id, product_family_id).await?;
        let mut preview = SignupBackfillPreview {
            plan_name: plan.plan_name.clone(),
            customers: 0,
            churned: 0,
            not_sold: 0,
            eligibility_rule: plan.eligibility.is_some(),
            not_eligible: 0,
            unfinished: u32::try_from(
                count_unfinished(&mut conn, tenant_id, product_family_id).await?,
            )
            .unwrap_or(u32::MAX),
        };
        let mut sold = BTreeSet::new();
        for count in counts {
            let customers = u32::try_from(count.customers).unwrap_or(u32::MAX);
            if count.churned {
                preview.churned += customers;
                if !include_churned {
                    continue;
                }
            }
            if plan.offer(&count.currency, &[]).is_err() {
                preview.not_sold += customers;
            } else {
                preview.customers += customers;
                sold.insert(count.currency);
            }
        }
        // The rule is checked as the backfill checks it, on what it reads of each customer.
        if let Some(rule) = plan.eligibility.as_ref() {
            let sold: Vec<String> = sold.into_iter().collect();
            let facts = planless_customer_facts(
                &mut conn,
                tenant_id,
                product_family_id,
                &sold,
                include_churned,
            )
            .await?;
            let not_eligible = facts
                .iter()
                .filter(|facts| {
                    let country = facts
                        .billing_address
                        .clone()
                        .and_then(|address| serde_json::from_value::<Address>(address).ok())
                        .and_then(|address| address.country)
                        .map(|country| country.code.clone());
                    eligibility::check(rule, &CustomerFacts::new(country, &facts.custom_properties))
                        .is_err()
                })
                .count();
            preview.not_eligible = u32::try_from(not_eligible).unwrap_or(u32::MAX);
            preview.customers = preview.customers.saturating_sub(preview.not_eligible);
        }
        Ok(preview)
    }

    /// The customers `signup_backfill_preview` counts, by id, and the plan version they start on.
    pub async fn signup_backfill_customers(
        &self,
        tenant_id: TenantId,
        product_family_id: ProductFamilyId,
        include_churned: bool,
    ) -> StoreResult<(PlanVersionId, Vec<CustomerId>)> {
        let plan = self.free_signup_plan(tenant_id, product_family_id).await?;
        let mut conn = self.store.get_conn().await?;
        let sold: Vec<String> = count_planless(&mut conn, tenant_id, product_family_id)
            .await?
            .into_iter()
            .map(|count| count.currency)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter(|currency| plan.offer(currency, &[]).is_ok())
            .collect();
        let customers = planless_customer_ids(
            &mut conn,
            tenant_id,
            product_family_id,
            &sold,
            include_churned,
        )
        .await?;
        Ok((plan.plan_version_id(), customers))
    }

    /// Starts `customer_id` on the free sign-up plan of `product_family_id`, `plan_version_id`
    /// still, unless archived, on a plan there by now, or left off by its rule. The plan writes no
    /// document, so nothing is sent. The subscription is recorded as `batch_job`'s with it.
    pub async fn backfill_signup_plan(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        product_family_id: ProductFamilyId,
        plan_version_id: PlanVersionId,
        batch_job: Option<BatchJobId>,
        customer_id: CustomerId,
    ) -> StoreResult<BackfillOutcome> {
        let plan = self.free_signup_plan(tenant_id, product_family_id).await?;
        if plan.plan_version_id() != plan_version_id {
            return Err(Report::new(StoreError::InvalidArgument(
                "The sign-up plan changed since these customers were previewed".to_owned(),
            )));
        }
        let customer = self
            .store
            .find_customer_by_id(customer_id, tenant_id)
            .await?;
        if plan
            .eligibility
            .as_ref()
            .is_some_and(|rule| eligibility::check(rule, &CustomerFacts::of(&customer)).is_err())
        {
            return Ok(BackfillOutcome::NotEligible);
        }
        let offer = plan.offer(&customer.currency, &[])?;
        let inserted = self
            .store
            .transaction(|conn| {
                let (actor, customer, offer) = (&actor, &customer, &offer);
                async move {
                    // The lock every subscription insert takes: none commits for the customer
                    // between the check and this one.
                    lock_customer_tx(conn, tenant_id, customer.id).await?;
                    if !is_planless(conn, tenant_id, product_family_id, customer.id).await? {
                        return Ok(Vec::new());
                    }
                    let inserted = self
                        .subscribe_on_signup_tx(
                            conn,
                            actor,
                            tenant_id,
                            customer,
                            offer,
                            Some(SubscriptionOrigin::SignupPlanBackfill),
                        )
                        .await?;
                    if let Some(batch_job_id) = batch_job {
                        let entities: Vec<_> = inserted
                            .iter()
                            .map(|created| BatchJobEntityRowNew {
                                batch_job_id,
                                tenant_id,
                                entity_type: "subscription".to_owned(),
                                entity_id: created.id.as_uuid(),
                            })
                            .collect();
                        BatchJobEntityRowNew::insert_batch(conn, &entities).await?;
                    }
                    Ok(inserted)
                }
                .scope_boxed()
            })
            .await?;
        let Some(created) = inserted.first() else {
            return Ok(BackfillOutcome::Skipped);
        };
        let id = created.id;
        self.services
            .handle_post_insertion(&actor, self.store.eventbus.clone(), &inserted)
            .await?;
        Ok(BackfillOutcome::Subscribed(id))
    }
}
