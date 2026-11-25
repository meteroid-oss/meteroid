//! A customer created asking for it starts on its product family's sign-up plan.
use crate::StoreResult;
use crate::domain::PaymentMethodsConfig;
use crate::domain::eligibility::{self, CustomerFacts, Eligibility};
use crate::domain::enums::SubscriptionActivationCondition;
use crate::domain::plan_options::{PlanOption, PlanOptionRef};
use crate::domain::price_cells::{resolve_option, sold_options};
use crate::domain::usage_levels::{ChosenLevel, starting_levels};
use crate::domain::{
    ComponentParameterization, CreateSubscription, CreateSubscriptionComponents,
    CreatedSubscription, SubscriptionNew,
};
use crate::domain::{Customer, CustomerNew};
use crate::errors::StoreError;
use crate::repositories::ProductFamilyJourneysInterface;
use crate::repositories::customers::CustomersInterfaceAuto;
use crate::repositories::price_components::PriceComponentInterface;
use crate::services::ServicesEdge;
use common_domain::actor::Actor;
use common_domain::ids::{CustomerId, PlanVersionId, ProductFamilyId, TenantId};
use common_eventbus::Event;
use diesel_models::invoicing_entities::InvoicingEntityProvidersRow;
use diesel_models::plan_versions::PlanVersionFilter;
use diesel_models::plans::PlanRow;
use diesel_models::product_family_journeys::ProductFamilyJourneyRow;
use error_stack::Report;
use scoped_futures::ScopedFutureExt;

/// What a sign-up subscribes to, resolved before the customer is created.
#[derive(Clone, Debug)]
pub struct SignupOffer {
    pub product_family_id: ProductFamilyId,
    pub plan_version_id: PlanVersionId,
    /// The plan option it starts on.
    pub option: Option<PlanOption>,
    /// The customer's.
    pub currency: String,
    /// Who may get the plan; checked on the customer it subscribes.
    pub eligibility: Option<Eligibility>,
    /// The level each usage-levels charge starts on (`starting_levels`).
    pub levels: Vec<ComponentParameterization>,
}

fn invalid(message: String) -> Report<StoreError> {
    Report::new(StoreError::InvalidArgument(message))
}

impl ServicesEdge {
    /// The sign-up plan of `product_family_id`, or of the only family that has one, as a customer
    /// in `currency` takes it: on the journey's option, else the first sold in that currency.
    /// Refused when that option is not sold in it. Its usage-levels charges start on their lowest.
    pub async fn signup_offer(
        &self,
        tenant_id: TenantId,
        product_family_id: Option<ProductFamilyId>,
        currency: &str,
    ) -> StoreResult<SignupOffer> {
        self.signup_offer_at_levels(tenant_id, product_family_id, currency, &[])
            .await
    }

    /// `signup_offer`, each usage-levels charge on the level `levels` picks for it, else its
    /// lowest: the level the pricing page reads the plan's price from. A level not of the cell
    /// sold is refused.
    pub async fn signup_offer_at_levels(
        &self,
        tenant_id: TenantId,
        product_family_id: Option<ProductFamilyId>,
        currency: &str,
        levels: &[ChosenLevel],
    ) -> StoreResult<SignupOffer> {
        let mut conn = self.store.get_conn().await?;
        let journey = match product_family_id {
            Some(id) => self.store.get_product_family_journey(tenant_id, id).await?,
            None => {
                let mut with_plan: Vec<_> = ProductFamilyJourneyRow::list(&mut conn, tenant_id)
                    .await?
                    .into_iter()
                    .filter(|journey| journey.signup_plan_id.is_some())
                    .collect();
                match (with_plan.pop(), with_plan.is_empty()) {
                    (Some(only), true) => only.into(),
                    (Some(_), false) => {
                        return Err(invalid(
                            "Several product lines have a sign-up plan: name the product line"
                                .to_owned(),
                        ));
                    }
                    (None, _) => {
                        return Err(invalid("No product line has a sign-up plan".to_owned()));
                    }
                }
            }
        };
        let plan_id = journey
            .signup_plan_id
            .ok_or_else(|| invalid("This product line has no sign-up plan".to_owned()))?;
        let plan = PlanRow::get_with_version_by_id(
            &mut conn,
            plan_id,
            tenant_id,
            PlanVersionFilter::Active,
        )
        .await?;
        if plan.plan.pricing_sales_led {
            return Err(invalid(
                "The sign-up plan is sold by contacting sales".to_owned(),
            ));
        }
        let eligibility = Eligibility::from_row(plan.plan.eligibility.clone());
        let version = plan
            .version
            .ok_or_else(|| invalid("Publish the sign-up plan first".to_owned()))?;
        let components = self
            .store
            .list_price_components_with_conn(&mut conn, version.id, tenant_id)
            .await?;
        crate::repositories::product_family_journeys::signup_ready(
            plan.plan.plan_type,
            &version,
            &components,
        )
        .map_err(|message| invalid(message.to_owned()))?;
        let currency = currency.trim().to_ascii_uppercase();
        let options = crate::services::subscriptions::currency::plan_options_of(
            &mut conn,
            tenant_id,
            &[version.id],
        )
        .await?
        .remove(&version.id)
        .unwrap_or_default();
        // A plan with no price at all is free in every currency.
        let unpriced = components
            .iter()
            .all(|c| c.prices.is_empty() && c.legacy_pricing.is_none());
        // Never a hidden option unless the journey names it: sign-up is public.
        let default = || {
            sold_options(&options, &components, &currency)
                .into_iter()
                .find(|option| !option.hidden)
        };
        let option = match journey.signup_option.as_deref() {
            // An alias the version lost falls back to its default, as at a plan change.
            Some(alias) => resolve_option(&options, Some(&PlanOptionRef::Alias(alias.into())))
                .ok()
                .or_else(default),
            None => default(),
        }
        .or_else(|| unpriced.then(|| options.first()).flatten())
        .cloned();
        let sold = unpriced
            || option.as_ref().is_some_and(|option| {
                crate::services::subscriptions::currency::pricing(option, &currency)
                    .sells(&components)
            });
        if !sold {
            return Err(invalid(format!(
                "The sign-up plan is not offered in {currency}"
            )));
        }
        if !currency.eq_ignore_ascii_case(&version.currency)
            && crate::repositories::plans::load_plan_minimum(&mut conn, version.id)
                .await?
                .is_some()
        {
            return Err(invalid(format!(
                "The sign-up plan's minimum commitment is in {}: it cannot sign up customers in {currency}",
                version.currency
            )));
        }
        let mut sold_cells = components;
        if let Some(option) = option.as_ref() {
            crate::services::subscriptions::currency::pricing(option, &currency)
                .narrow(&mut sold_cells);
        }
        let levels = starting_levels(&sold_cells, levels).map_err(Report::new)?;
        Ok(SignupOffer {
            product_family_id: journey.product_family_id,
            plan_version_id: version.id,
            option,
            currency,
            eligibility,
            levels,
        })
    }

    /// Creates the customer on the sign-up `offer`, both or neither.
    pub async fn create_customer_on_signup(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        customer: CustomerNew,
        offer: SignupOffer,
    ) -> StoreResult<Customer> {
        let customer = self.store.prepare_customer(customer, tenant_id).await?;
        let (customer, inserted) = self
            .store
            .transaction(|conn| {
                let actor = &actor;
                async move {
                    let customer = self
                        .store
                        .insert_customer_tx(conn, actor, tenant_id, customer)
                        .await?;
                    let inserted = self
                        .subscribe_on_signup_tx(conn, actor, tenant_id, &customer, &offer)
                        .await?;
                    Ok((customer, inserted))
                }
                .scope_boxed()
            })
            .await?;
        let _ = self
            .store
            .eventbus
            .publish(Event::customer_created(
                actor.clone(),
                customer.id,
                tenant_id,
            ))
            .await;
        self.services
            .handle_post_insertion(&actor, self.store.eventbus.clone(), &inserted)
            .await?;
        Ok(customer)
    }

    /// Subscribes an existing customer to `offer`.
    pub async fn subscribe_to_signup_plan(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        customer_id: CustomerId,
        offer: SignupOffer,
    ) -> StoreResult<CreatedSubscription> {
        let customer = self
            .store
            .find_customer_by_id(customer_id, tenant_id)
            .await?;
        let inserted = self
            .store
            .transaction(|conn| {
                let actor = &actor;
                async move {
                    self.subscribe_on_signup_tx(conn, actor, tenant_id, &customer, &offer)
                        .await
                }
                .scope_boxed()
            })
            .await?;
        self.services
            .handle_post_insertion(&actor, self.store.eventbus.clone(), &inserted)
            .await?;
        inserted
            .into_iter()
            .next()
            .ok_or_else(|| Report::new(StoreError::InsertError))
    }

    /// The subscription from today, with the plan's trial, charged automatically when the
    /// customer's invoicing entity takes cards or direct debits, else invoiced.
    async fn subscribe_on_signup_tx(
        &self,
        conn: &mut crate::store::PgConn,
        actor: &Actor,
        tenant_id: TenantId,
        customer: &Customer,
        offer: &SignupOffer,
    ) -> StoreResult<Vec<CreatedSubscription>> {
        let providers = InvoicingEntityProvidersRow::resolve_providers_by_id(
            conn,
            customer.invoicing_entity_id,
            tenant_id,
        )
        .await?;
        let online = providers.card_provider.is_some() || providers.direct_debit_provider.is_some();
        if !customer.currency.eq_ignore_ascii_case(&offer.currency) {
            return Err(invalid(format!(
                "The sign-up plan is offered in {}, the customer pays in {}",
                offer.currency, customer.currency
            )));
        }
        if let Some(rule) = offer.eligibility.as_ref() {
            eligibility::check(rule, &CustomerFacts::of(customer))
                .map_err(|why| invalid(why.message()))?;
        }
        let batch = vec![CreateSubscription {
            option: offer
                .option
                .as_ref()
                .map(|option| PlanOptionRef::Id(option.id)),
            subscription: SubscriptionNew {
                customer_id: customer.id,
                plan_version_id: offer.plan_version_id,
                net_terms: None,
                invoice_memo: None,
                invoice_threshold: None,
                start_date: common_clock::now().date_naive(),
                end_date: None,
                billing_start_date: None,
                activation_condition: SubscriptionActivationCondition::OnStart,
                trial_duration: None,
                billing_day_anchor: None,
                payment_methods_config: (!online).then_some(PaymentMethodsConfig::External),
                auto_advance_invoices: true,
                charge_automatically: online,
                purchase_order: None,
                backdate_invoices: false,
                skip_checkout_session: false,
                skip_past_invoices: false,
                usage_filter: None,
                custom_properties: serde_json::Value::Null,
            },
            price_components: (!offer.levels.is_empty()).then(|| CreateSubscriptionComponents {
                parameterized_components: offer.levels.clone(),
                overridden_components: vec![],
                extra_components: vec![],
                remove_components: vec![],
            }),
            add_ons: None,
            coupons: None,
            entitlements: vec![],
            minimum_override: None,
            drop_minimum: false,
        }];
        let context = self
            .services
            .gather_subscription_context(conn, &batch, tenant_id, &self.store.settings.crypt_key)
            .await?;
        let mut processed = Vec::new();
        for sub in self.services.build_subscription_details(&batch, &context)? {
            let payment = self.services.setup_payment_provider(&sub, &context)?;
            processed.push(
                self.services
                    .process_subscription(&sub, &payment, &context, tenant_id, None)?,
            );
        }
        self.services
            .persist_subscriptions(
                conn,
                actor,
                &processed,
                tenant_id,
                &self.store.settings.public_url,
            )
            .await
    }
}
