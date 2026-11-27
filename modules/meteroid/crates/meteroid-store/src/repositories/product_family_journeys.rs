//! A product line's self-serve journey, its defaults until saved.
use crate::StoreResult;
use crate::domain::plan_options::PlanOptionRef;
use crate::domain::price_cells::{priced_options, resolve_option};
use crate::domain::price_components::{PriceComponent, charges_nothing};
use crate::domain::product_family_journeys::ProductFamilyJourney;
use crate::errors::StoreError;
use crate::repositories::price_components::PriceComponentInterface;
use crate::store::Store;
use common_domain::ids::{ProductFamilyId, TenantId};
use diesel_models::enums::PlanTypeEnum;
use diesel_models::plan_versions::PlanVersionFilter;
use diesel_models::plans::PlanRow;
use diesel_models::product_families::ProductFamilyRow;
use diesel_models::product_family_journeys::ProductFamilyJourneyRow;
use error_stack::Report;

#[async_trait::async_trait]
pub trait ProductFamilyJourneysInterface {
    async fn get_product_family_journey(
        &self,
        tenant_id: TenantId,
        product_family_id: ProductFamilyId,
    ) -> StoreResult<ProductFamilyJourney>;

    /// A sign-up plan is one of the family's, free or starting with a trial, its option sold in
    /// some currency (a sign-up subscribes in the customer's).
    async fn save_product_family_journey(
        &self,
        tenant_id: TenantId,
        journey: ProductFamilyJourney,
    ) -> StoreResult<ProductFamilyJourney>;
}

fn invalid(message: &str) -> Report<StoreError> {
    Report::new(StoreError::InvalidArgument(message.to_owned()))
}

#[async_trait::async_trait]
impl ProductFamilyJourneysInterface for Store {
    async fn get_product_family_journey(
        &self,
        tenant_id: TenantId,
        product_family_id: ProductFamilyId,
    ) -> StoreResult<ProductFamilyJourney> {
        let mut conn = self.get_conn().await?;
        ProductFamilyRow::find_by_id(&mut conn, product_family_id, tenant_id).await?;
        Ok(
            ProductFamilyJourneyRow::find(&mut conn, tenant_id, product_family_id)
                .await?
                .map_or_else(
                    || ProductFamilyJourney::default_for(product_family_id),
                    Into::into,
                ),
        )
    }

    async fn save_product_family_journey(
        &self,
        tenant_id: TenantId,
        journey: ProductFamilyJourney,
    ) -> StoreResult<ProductFamilyJourney> {
        let mut conn = self.get_conn().await?;
        ProductFamilyRow::find_by_id(&mut conn, journey.product_family_id, tenant_id).await?;
        let mut signup_option = journey.signup_option.clone();
        if let Some(plan_id) = journey.signup_plan_id {
            let plan = PlanRow::get_with_version_by_id(
                &mut conn,
                plan_id,
                tenant_id,
                PlanVersionFilter::Active,
            )
            .await?;
            if plan.plan.product_family_id != journey.product_family_id {
                return Err(invalid("Pick a sign-up plan of this product line"));
            }
            if plan.plan.pricing_sales_led {
                return Err(invalid(
                    "Pick a self-serve sign-up plan, not one sold by sales",
                ));
            }
            let version = plan
                .version
                .ok_or_else(|| invalid("Publish the sign-up plan first"))?;
            let components = self
                .list_price_components_with_conn(&mut conn, version.id, tenant_id)
                .await?;
            signup_ready(
                plan.plan.plan_type,
                crate::services::trials::resolve::starts_free_tx(
                    &mut conn,
                    tenant_id,
                    version.plan_id,
                )
                .await?,
                &components,
            )
            .map_err(invalid)?;
            if let Some(alias) = signup_option.as_deref() {
                let options = crate::services::subscriptions::currency::plan_options_of(
                    &mut conn,
                    tenant_id,
                    &[version.id],
                )
                .await?
                .remove(&version.id)
                .unwrap_or_default();
                let option = resolve_option(&options, Some(&PlanOptionRef::Alias(alias.into())))?;
                // A plan with no price at all is free in every currency.
                if components.iter().any(|c| !c.prices.is_empty())
                    && !priced_options(&options, &components)
                        .iter()
                        .any(|(_, id)| *id == option.id)
                {
                    return Err(invalid("The sign-up plan does not sell this option"));
                }
                signup_option = Some(option.alias.clone());
            }
        } else if signup_option.is_some() {
            return Err(invalid("Pick a sign-up plan first"));
        }
        Ok(ProductFamilyJourneyRow {
            product_family_id: journey.product_family_id,
            tenant_id,
            signup_plan_id: journey.signup_plan_id,
            signup_option,
            period_switch: journey.period_switch,
            self_serve_downgrade: journey.self_serve_downgrade,
            updated_at: common_clock::now(),
        }
        .upsert(&mut conn)
        .await?
        .into())
    }
}

/// A new customer has no payment method: they start on a free plan (typed free, or priced at
/// nothing), or on a trial billed nothing until it ends.
pub(crate) fn signup_ready(
    plan_type: PlanTypeEnum,
    free_trial: bool,
    components: &[PriceComponent],
) -> Result<(), &'static str> {
    if plan_type == PlanTypeEnum::Free || charges_nothing(components) || free_trial {
        Ok(())
    } else {
        Err("Pick a free plan or one with a free trial: a new customer has no payment method yet")
    }
}
