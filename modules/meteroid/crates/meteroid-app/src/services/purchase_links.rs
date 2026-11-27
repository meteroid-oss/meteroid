//! Purchases started from public purchase links: input checks, a proof of work, rate limits,
//! then a customer and a SelfServe checkout session. The buyer finishes on the checkout page.

use common_domain::actor::Actor;
use common_domain::country::CountryCode;
use common_domain::ids::{AliasOr, BaseId, CheckoutSessionId, CustomerId, ProductId, TenantId};
use error_stack::Report;
use license_protocol::protocol::PurchaseRequest;
use meteroid_store::Store;
use meteroid_store::domain::checkout_sessions::{
    CheckoutSession, CheckoutSessionStatus, CheckoutType, CreateCheckoutSession,
};
use meteroid_store::domain::eligibility::{self, CustomerFacts};
use meteroid_store::domain::enums::{CustomerType, SubscriptionFeeBillingPeriod};
use meteroid_store::domain::plan_options::{PlanOption, PlanOptionRef};
use meteroid_store::domain::price_cells::{OptionPricing, currencies, resolve_option};
use meteroid_store::domain::purchase_links::{
    AllowedBuyerTypes, InstanceId, LivePurchaseLink, PurchaseAttemptRestart, PurchaseLink,
    PurchaseLinkSession, PurchaseLinkSessionNew, TaxIdCollection,
};
use meteroid_store::domain::subscription_components::{
    ComponentParameterization, ComponentParameters, CreateSubscriptionComponents, SubscriptionFee,
};
use meteroid_store::domain::usage_levels::{self, ChosenLevel};
use meteroid_store::domain::{
    Address, CustomerNew, CustomerPatch, CustomerTaxStatus, PriceComponent,
    is_complete_billing_information,
};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::billable_metrics::BillableMetricInterface;
use meteroid_store::repositories::checkout_sessions::CheckoutSessionsInterface;
use meteroid_store::repositories::credit_units::CreditUnitsInterface;
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterfaceAuto;
use meteroid_store::repositories::portal_tokens::PortalTokensInterface;
use meteroid_store::repositories::price_components::PriceComponentInterfaceAuto;
use meteroid_store::repositories::purchase_links::PurchaseLinksInterface;
use meteroid_store::repositories::{CustomersInterface, PlanOptionInterface};
use meteroid_store::services::MAX_DECLINED_CHECKOUT_ATTEMPTS;
use rate_limiter::{RateLimitConfig, RateLimiter};
use secrecy::SecretString;
use validator::ValidateEmail;

use crate::services::proof_of_work::ProofOfWork;

const CHECKOUT_EXPIRY_HOURS: u32 = 1;
const MAX_EMAIL_LENGTH: usize = 254;
const MAX_NAME_LENGTH: usize = 200;
const MAX_ADDRESS_FIELD_LENGTH: usize = 200;
const MAX_TAX_ID_LENGTH: usize = 64;
const MAX_LICENSE_REQUEST_LENGTH: usize = 4096;
const ATTEMPT_ID_LENGTH: std::ops::RangeInclusive<usize> = 16..=64;
/// Redis buckets on StartPurchase; without Redis only the proof of work applies.
const STARTS_PER_IP_PER_HOUR: u32 = 10;
const STARTS_PER_LINK_PER_HOUR: u32 = 300;

#[derive(Debug, thiserror::Error)]
pub enum PurchaseError {
    #[error("This purchase link is not available")]
    NotFound,
    #[error("{0}")]
    InvalidInput(String),
    /// The plan is not for this buyer, as they entered themselves.
    #[error("{0}")]
    Ineligible(String),
    /// The coupon code cannot be redeemed on this purchase.
    #[error("{0}")]
    CouponCode(String),
    #[error("Verification failed, please try again")]
    ProofOfWorkFailed,
    #[error("Too many purchase attempts, please try again later")]
    Throttled,
    #[error("This purchase can't be started, please contact the seller")]
    Unavailable,
    #[error("Internal error")]
    Internal(Report<StoreError>),
}

impl From<Report<StoreError>> for PurchaseError {
    fn from(value: Report<StoreError>) -> Self {
        PurchaseError::Internal(value)
    }
}

impl From<PurchaseError> for tonic::Status {
    fn from(value: PurchaseError) -> Self {
        let message = value.to_string();
        match value {
            PurchaseError::NotFound => tonic::Status::not_found(message),
            PurchaseError::InvalidInput(_)
            | PurchaseError::Ineligible(_)
            | PurchaseError::CouponCode(_)
            | PurchaseError::ProofOfWorkFailed => tonic::Status::invalid_argument(message),
            PurchaseError::Throttled => tonic::Status::resource_exhausted(message),
            PurchaseError::Unavailable => tonic::Status::failed_precondition(message),
            PurchaseError::Internal(e) => {
                log::error!("Purchase failed: {e:?}");
                tonic::Status::internal(message)
            }
        }
    }
}

pub enum Buyer {
    Company {
        name: String,
    },
    Individual {
        first_name: String,
        last_name: String,
    },
}

/// The billing address apart from its country.
#[derive(Debug, Clone, Default)]
pub struct BillingAddressInput {
    pub line1: Option<String>,
    pub line2: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub zip_code: Option<String>,
}

pub struct StartPurchase {
    pub token: String,
    pub email: String,
    pub buyer: Buyer,
    pub country: String,
    pub address: BillingAddressInput,
    /// Companies only.
    pub vat_number: Option<String>,
    /// Companies only.
    pub legal_number: Option<String>,
    pub pow_solution: String,
    pub attempt_id: String,
    pub license_request: Option<String>,
    pub client_ip: Option<String>,
    /// Entered by the buyer; none: the link's own, if any.
    pub coupon_code: Option<String>,
    /// The level picked for each usage-levels charge; a charge left out starts on its lowest.
    pub levels: Vec<ChosenLevel>,
}

#[derive(Debug, Clone)]
pub struct StartedPurchase {
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub checkout_session_id: CheckoutSessionId,
    pub checkout_url: String,
}

/// A plan component as a subscription started from the link gets it.
pub struct OfferComponent {
    pub name: String,
    pub product_id: Option<ProductId>,
    pub period: SubscriptionFeeBillingPeriod,
    /// At its lowest level for a usage-levels charge.
    pub fee: SubscriptionFee,
    /// A usage-levels charge's fee at each of its levels, fewest units first; empty for any other.
    pub levels: Vec<SubscriptionFee>,
    /// What a usage-levels charge's units count, as its metric names them (plural).
    pub unit: Option<String>,
}

pub struct PurchaseOffer {
    pub link: LivePurchaseLink,
    pub components: Vec<OfferComponent>,
    /// The currency sold in.
    pub currency: String,
    /// The name of the plan option it sells, when the plan has several.
    pub option_name: Option<String>,
    /// The coupon the link applies when the buyer enters none.
    pub coupon_code: Option<String>,
    /// ALTCHA challenge JSON.
    pub pow_challenge: String,
    /// The installation a license request names; the license goes to it.
    pub installation_id: Option<InstanceId>,
}

#[derive(Clone)]
pub struct PurchaseLinks {
    store: Store,
    public_url: String,
    pow: ProofOfWork,
    ip_limiter: RateLimiter,
    link_limiter: RateLimiter,
}

impl PurchaseLinks {
    pub fn new(
        store: Store,
        jwt_secret: SecretString,
        public_url: String,
        rate_limiter: &RateLimiter,
    ) -> Self {
        Self {
            store,
            pow: ProofOfWork::new(&jwt_secret),
            public_url: public_url.trim_end_matches('/').to_string(),
            ip_limiter: rate_limiter
                .with_config(hourly_bucket("purchase:start:ip", STARTS_PER_IP_PER_HOUR)),
            link_limiter: rate_limiter.with_config(hourly_bucket(
                "purchase:start:link",
                STARTS_PER_LINK_PER_HOUR,
            )),
        }
    }

    pub fn purchase_link_url(&self, token: &str) -> String {
        format!("{}/purchase/{token}", self.public_url)
    }

    pub async fn get_offer(
        &self,
        token: &str,
        license_request: Option<&str>,
    ) -> Result<PurchaseOffer, PurchaseError> {
        let installation_id = license_request
            .filter(|r| !r.is_empty())
            .map(verify_license_request)
            .transpose()?
            .map(|(instance_id, _)| instance_id);

        let link = self
            .store
            .find_live_purchase_link(token)
            .await?
            .ok_or(PurchaseError::NotFound)?;
        let components = self
            .store
            .list_price_components(link.version.id, link.link.tenant_id)
            .await?;
        let options = self
            .store
            .list_plan_options(link.link.tenant_id, link.version.id)
            .await?;
        let sold = sold_option(&link, &options, &components)?;
        let option_name = (options.len() > 1).then(|| sold.option_name.clone());

        let components = self
            .offer_components(&link, &sold, components)
            .await
            .unwrap_or_else(|e| {
                log::warn!(
                    "Could not resolve the prices of purchase link {}: {e:?}",
                    link.link.id
                );
                vec![]
            });
        let currency = sold.pricing.currency.clone();
        let coupon_code = link.link.coupon_code.clone();

        let pow_challenge = serde_json::to_string(&self.pow.create_challenge(&link.link.token))
            .map_err(|e| {
                PurchaseError::Internal(Report::new(StoreError::SerdeError("pow".into(), e)))
            })?;

        Ok(PurchaseOffer {
            link,
            components,
            currency,
            option_name,
            coupon_code,
            pow_challenge,
            installation_id,
        })
    }

    /// Resolved like the subscription a purchase creates: the plan's components, default
    /// parameters, a usage-levels charge on its lowest level and priced at each of the others.
    async fn offer_components(
        &self,
        link: &LivePurchaseLink,
        sold: &SoldOption,
        mut components: Vec<PriceComponent>,
    ) -> Result<Vec<OfferComponent>, Report<StoreError>> {
        let tenant_id = link.link.tenant_id;
        sold.pricing.narrow(&mut components);
        let units = self
            .store
            .get_credit_units_by_ids(
                tenant_id,
                components
                    .iter()
                    .flat_map(PriceComponent::credit_unit_ids)
                    .collect(),
            )
            .await?;
        let at_level = |included: u64| ComponentParameters {
            initial_slot_count: None,
            billing_period: None,
            committed_capacity: Some(included),
        };
        let mut offered: Vec<OfferComponent> = components
            .iter()
            .filter(|c| !c.is_included_only())
            .map(|c| {
                let sizes = usage_levels::sizes(&usage_levels::levels(c));
                let params = sizes.first().map(|lowest| at_level(*lowest));
                let resolved = c.resolve_fee(&units, params.as_ref())?;
                let levels = sizes
                    .iter()
                    .map(|included| {
                        c.resolve_fee(&units, Some(&at_level(*included)))
                            .map(|level| level.fee)
                    })
                    .collect::<Result<_, StoreError>>()?;
                Ok(OfferComponent {
                    name: c.name.clone(),
                    product_id: c.product_id,
                    period: resolved.period,
                    fee: resolved.fee,
                    levels,
                    unit: None,
                })
            })
            .collect::<Result<_, StoreError>>()
            .map_err(Report::new)?;

        // The levels read in their metric's unit ("€250K", "250K requests").
        let level_metric = |component: &OfferComponent| match component.levels.first() {
            Some(SubscriptionFee::Capacity { metric_id, .. }) => Some(*metric_id),
            _ => None,
        };
        let metric_ids: Vec<_> = offered.iter().filter_map(level_metric).collect();
        if !metric_ids.is_empty() {
            let metrics = self
                .store
                .find_billable_metrics_by_ids(&metric_ids, tenant_id)
                .await?;
            for component in &mut offered {
                component.unit = level_metric(&*component).and_then(|id| {
                    metrics
                        .iter()
                        .find(|m| m.id == id)
                        .and_then(|m| m.unit_plural.clone())
                });
            }
        }
        Ok(offered)
    }

    /// Starts a purchase, or resumes the one of the same `attempt_id` (see the arms below).
    pub async fn start(&self, input: StartPurchase) -> Result<StartedPurchase, PurchaseError> {
        let buyer = validate(&input)?;
        let pow = self
            .pow
            .verify(&input.pow_solution, &input.token)
            .map_err(|_| PurchaseError::ProofOfWorkFailed)?;
        if !self
            .store
            .spend_pow_challenge(&pow.challenge, pow.expires_at.naive_utc())
            .await?
        {
            return Err(PurchaseError::ProofOfWorkFailed);
        }

        if let Some(ip) = input.client_ip.as_deref()
            && !self.ip_limiter.check(ip).await.is_allowed()
        {
            log::warn!("Purchase throttled (ip)");
            return Err(PurchaseError::Throttled);
        }

        let link = self
            .store
            .find_live_purchase_link(&input.token)
            .await?
            .ok_or(PurchaseError::NotFound)?;
        let tenant_id = link.link.tenant_id;
        // Purchase customers are created under the default invoicing entity.
        let requires_billing_information = self
            .store
            .get_invoicing_entity(tenant_id, None)
            .await?
            .require_billing_information;
        check_link_options(&link.link, &buyer, requires_billing_information)?;
        let components = self
            .store
            .list_price_components(link.version.id, tenant_id)
            .await?;
        let options = self
            .store
            .list_plan_options(tenant_id, link.version.id)
            .await?;
        let sold = sold_option(&link, &options, &components)?;
        let levels = purchase_levels(&sold, components, &input.levels)?;
        let facts = CustomerFacts {
            country: Some(input.country.trim().to_ascii_uppercase()),
            ..Default::default()
        };
        if let Some(rule) = link.plan.eligibility.as_ref() {
            eligibility::check(rule, &facts)
                .map_err(|why| PurchaseError::Ineligible(why.message()))?;
        }
        let coupon_code = self
            .coupon_code(&link, &sold, &facts, input.coupon_code.as_deref())
            .await?;

        let installation = input
            .license_request
            .as_deref()
            .filter(|r| !r.is_empty())
            .map(verify_license_request)
            .transpose()?;

        let attempt = self
            .store
            .find_purchase_attempt(tenant_id, link.link.id, &input.attempt_id)
            .await?;

        let Some(attempt) = attempt else {
            self.check_link_bucket(&link).await?;
            return self
                .start_new(
                    &link,
                    &sold,
                    &buyer,
                    coupon_code.as_deref(),
                    &levels,
                    input.attempt_id,
                    installation,
                )
                .await;
        };

        let checkout = match attempt.checkout_session_id {
            Some(id) => Some(self.store.get_checkout_session(tenant_id, id).await?),
            None => None,
        };
        // Another coupon code or level than the open checkout's: a new checkout takes it, unless
        // a payment of the open one is in flight.
        let checkout = match checkout {
            Some(open)
                if open.can_complete()
                    && open.subscription_id.is_none()
                    && (open.coupon_code != coupon_code
                        || !same_levels(&open.component_params(), &levels)) =>
            {
                refuse_superseding(open.status)?;
                Some(
                    self.store
                        .cancel_checkout_session(tenant_id, open.id)
                        .await?,
                )
            }
            checkout => checkout,
        };
        match checkout {
            Some(checkout) if checkout.is_completed() || checkout.subscription_id.is_some() => Err(
                PurchaseError::InvalidInput("This purchase was already completed".to_string()),
            ),
            Some(checkout) if checkout.can_complete() => {
                self.update_customer(tenant_id, attempt.customer_id, &buyer)
                    .await?;
                self.started(tenant_id, attempt.customer_id, &checkout)
                    .await
            }
            Some(checkout) if checkout.status == CheckoutSessionStatus::AwaitingPayment => {
                Err(PurchaseError::InvalidInput(
                    "A payment for this purchase is being processed".to_string(),
                ))
            }
            previous => {
                let declines = self
                    .store
                    .count_purchase_declines(tenant_id, attempt.customer_id)
                    .await?;
                if declines >= MAX_DECLINED_CHECKOUT_ATTEMPTS {
                    log::warn!(
                        "Purchase attempt {} refused after {declines} declined payments",
                        attempt.id
                    );
                    return Err(PurchaseError::InvalidInput(
                        "This purchase can't be retried, please contact the seller".to_string(),
                    ));
                }
                self.check_link_bucket(&link).await?;
                self.restart(
                    &link,
                    &sold,
                    &attempt,
                    previous.map(|c| c.id),
                    &buyer,
                    coupon_code.as_deref(),
                    &levels,
                    installation,
                )
                .await
            }
        }
    }

    /// The coupon code the purchase redeems, as its coupon spells it: the buyer's, else the
    /// link's. One the buyer entered is taken only from a link accepting codes, or when it is
    /// the link's own.
    async fn coupon_code(
        &self,
        link: &LivePurchaseLink,
        sold: &SoldOption,
        facts: &CustomerFacts,
        entered: Option<&str>,
    ) -> Result<Option<String>, PurchaseError> {
        let entered = entered.map(str::trim).filter(|code| !code.is_empty());
        let own = link.link.coupon_code.as_deref();
        let Some(code) = entered.or(own) else {
            return Ok(None);
        };
        if code.chars().count() > meteroid_store::repositories::coupons::MAX_COUPON_CODE_LENGTH {
            return Err(PurchaseError::CouponCode(
                "This coupon code is not valid".to_string(),
            ));
        }
        let is_own = own.is_some_and(|own| own.eq_ignore_ascii_case(code));
        if !is_own && !link.link.allow_coupon_codes {
            return Err(PurchaseError::CouponCode(
                "Coupon codes are not accepted for this purchase".to_string(),
            ));
        }
        let coupon = self
            .store
            .check_coupon_code(
                link.link.tenant_id,
                code,
                link.plan.id,
                &sold.pricing.currency,
                facts,
            )
            .await?
            .map_err(PurchaseError::CouponCode)?;
        Ok(Some(coupon.code))
    }

    /// Only purchases that open a new checkout count against the link.
    async fn check_link_bucket(&self, link: &LivePurchaseLink) -> Result<(), PurchaseError> {
        if self
            .link_limiter
            .check(link.link.id.as_base62())
            .await
            .is_allowed()
        {
            Ok(())
        } else {
            log::warn!("Purchase throttled (link) on link {}", link.link.id);
            Err(PurchaseError::Throttled)
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn start_new(
        &self,
        link: &LivePurchaseLink,
        sold: &SoldOption,
        buyer: &ValidatedBuyer,
        coupon_code: Option<&str>,
        levels: &[ComponentParameterization],
        attempt_id: String,
        installation: Option<(InstanceId, String)>,
    ) -> Result<StartedPurchase, PurchaseError> {
        let tenant_id = link.link.tenant_id;
        let customer = self
            .store
            .insert_customer(
                Actor::System,
                new_customer(buyer, &sold.pricing.currency),
                tenant_id,
            )
            .await
            .map_err(refused_or_internal)?;
        let checkout = match self
            .create_checkout(link, sold, customer.id, coupon_code, levels)
            .await
        {
            Ok(checkout) => checkout,
            Err(e) => {
                self.archive_customer(tenant_id, customer.id).await;
                return Err(e);
            }
        };

        let recorded = self
            .store
            .insert_purchase_link_session(PurchaseLinkSessionNew {
                tenant_id,
                purchase_link_id: link.link.id,
                customer_id: customer.id,
                checkout_session_id: checkout.id,
                attempt_id,
                license_instance_id: installation.as_ref().map(|i| i.0.clone()),
                license_instance_public_key: installation.map(|i| i.1),
            })
            .await;

        match recorded {
            Ok(Ok(_)) => self.started(tenant_id, customer.id, &checkout).await,
            Ok(Err(_)) => {
                // Lost a race on the same attempt id.
                self.discard(tenant_id, Some(customer.id), checkout.id)
                    .await;
                Err(PurchaseError::ProofOfWorkFailed)
            }
            Err(e) => {
                self.discard(tenant_id, Some(customer.id), checkout.id)
                    .await;
                Err(e.into())
            }
        }
    }

    /// The attempt's checkout closed unpaid: a new checkout, for the same customer unless the
    /// cleanup archived it.
    async fn restart(
        &self,
        link: &LivePurchaseLink,
        sold: &SoldOption,
        attempt: &PurchaseLinkSession,
        previous_checkout: Option<CheckoutSessionId>,
        buyer: &ValidatedBuyer,
        coupon_code: Option<&str>,
        levels: &[ComponentParameterization],
        installation: Option<(InstanceId, String)>,
    ) -> Result<StartedPurchase, PurchaseError> {
        let tenant_id = link.link.tenant_id;
        let reuses_customer = attempt.cleaned_up_at.is_none();
        let customer_id = if reuses_customer {
            self.update_customer(tenant_id, attempt.customer_id, buyer)
                .await?;
            attempt.customer_id
        } else {
            self.store
                .insert_customer(
                    Actor::System,
                    new_customer(buyer, &sold.pricing.currency),
                    tenant_id,
                )
                .await
                .map_err(refused_or_internal)?
                .id
        };
        let created = (!reuses_customer).then_some(customer_id);
        let checkout = match self
            .create_checkout(link, sold, customer_id, coupon_code, levels)
            .await
        {
            Ok(checkout) => checkout,
            Err(e) => {
                if let Some(created) = created {
                    self.archive_customer(tenant_id, created).await;
                }
                return Err(e);
            }
        };

        let restarted = self
            .store
            .restart_purchase_attempt(
                tenant_id,
                attempt.id,
                previous_checkout,
                reuses_customer,
                PurchaseAttemptRestart {
                    customer_id,
                    checkout_session_id: checkout.id,
                    license_instance_id: installation.as_ref().map(|i| i.0.clone()),
                    license_instance_public_key: installation.map(|i| i.1),
                },
            )
            .await;

        match restarted {
            Ok(Ok(())) => self.started(tenant_id, customer_id, &checkout).await,
            Ok(Err(_)) => {
                self.discard(tenant_id, created, checkout.id).await;
                Err(PurchaseError::InvalidInput(
                    "This purchase changed meanwhile, please try again".to_string(),
                ))
            }
            Err(e) => {
                self.discard(tenant_id, created, checkout.id).await;
                Err(e.into())
            }
        }
    }

    async fn create_checkout(
        &self,
        link: &LivePurchaseLink,
        sold: &SoldOption,
        customer_id: CustomerId,
        coupon_code: Option<&str>,
        levels: &[ComponentParameterization],
    ) -> Result<CheckoutSession, PurchaseError> {
        Ok(self
            .store
            .create_checkout_session(CreateCheckoutSession {
                tenant_id: link.link.tenant_id,
                customer_id,
                plan_version_id: link.version.id,
                option: Some(sold.option_alias.clone()),
                billing_start_date: None,
                billing_day_anchor: None,
                net_terms: None,
                trial: Default::default(),
                end_date: None,
                auto_advance_invoices: true,
                charge_automatically: true,
                invoice_memo: None,
                invoice_threshold: None,
                purchase_order: None,
                payment_methods_config: None,
                components: (!levels.is_empty()).then(|| CreateSubscriptionComponents {
                    parameterized_components: levels.to_vec(),
                    overridden_components: vec![],
                    extra_components: vec![],
                    remove_components: vec![],
                }),
                add_ons: None,
                coupon_code: coupon_code.map(str::to_owned),
                coupon_ids: vec![],
                expires_in_hours: Some(CHECKOUT_EXPIRY_HOURS),
                metadata: Some(serde_json::json!({
                    "purchase_link_id": link.link.id.as_base62(),
                })),
                checkout_type: CheckoutType::SelfServe,
                subscription_id: None,
                change_date: None,
                success_url: link.link.success_url.clone(),
                cancel_url: link.link.cancel_url.clone(),
                resource_id: None,
            })
            .await?)
    }

    async fn update_customer(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        buyer: &ValidatedBuyer,
    ) -> Result<(), PurchaseError> {
        self.store
            .patch_customer(Actor::System, tenant_id, customer_patch(customer_id, buyer))
            .await
            .map_err(refused_or_internal)?;
        Ok(())
    }

    /// The checkout token lives as long as the checkout it opens.
    async fn started(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        checkout: &CheckoutSession,
    ) -> Result<StartedPurchase, PurchaseError> {
        let token = self
            .store
            .checkout_portal_token(tenant_id, checkout.id, false)
            .await?;
        Ok(StartedPurchase {
            tenant_id,
            customer_id,
            checkout_session_id: checkout.id,
            checkout_url: format!("{}/checkout?token={token}", self.public_url),
        })
    }

    async fn archive_customer(&self, tenant_id: TenantId, customer_id: CustomerId) {
        if let Err(e) = self
            .store
            .archive_customer(Actor::System, tenant_id, AliasOr::Id(customer_id))
            .await
        {
            log::warn!("Could not archive unused purchase customer {customer_id}: {e:?}");
        }
    }

    async fn discard(
        &self,
        tenant_id: TenantId,
        customer_id: Option<CustomerId>,
        checkout_session_id: CheckoutSessionId,
    ) {
        if let Err(e) = self
            .store
            .cancel_checkout_session(tenant_id, checkout_session_id)
            .await
        {
            log::warn!("Could not cancel duplicate checkout {checkout_session_id}: {e:?}");
        }
        if let Some(customer_id) = customer_id {
            self.archive_customer(tenant_id, customer_id).await;
        }
    }
}

struct ValidatedBuyer {
    email: String,
    customer_type: CustomerType,
    name: String,
    first_name: Option<String>,
    last_name: Option<String>,
    address: Address,
    vat_number: Option<String>,
    legal_number: Option<String>,
}

/// Trimmed, `None` when blank.
fn optional_field(
    value: Option<&String>,
    max: usize,
    what: &str,
) -> Result<Option<String>, PurchaseError> {
    match value.map(|v| v.trim()).filter(|v| !v.is_empty()) {
        Some(v) if v.chars().count() > max => Err(PurchaseError::InvalidInput(format!(
            "{what} is too long (at most {max} characters)"
        ))),
        v => Ok(v.map(str::to_string)),
    }
}

fn validate(input: &StartPurchase) -> Result<ValidatedBuyer, PurchaseError> {
    let invalid = |msg: &str| PurchaseError::InvalidInput(msg.to_string());

    let email = input.email.trim().to_string();
    if email.len() > MAX_EMAIL_LENGTH || !email.validate_email() {
        return Err(invalid("A valid email address is required"));
    }

    let country = CountryCode::parse_as_opt(&input.country.trim().to_ascii_uppercase())
        .ok_or_else(|| invalid("A valid country is required"))?;

    if !ATTEMPT_ID_LENGTH.contains(&input.attempt_id.len())
        || !input
            .attempt_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(invalid("Invalid purchase attempt, reload the page"));
    }
    if input
        .license_request
        .as_ref()
        .is_some_and(|r| r.len() > MAX_LICENSE_REQUEST_LENGTH)
    {
        return Err(invalid("The installation request is invalid"));
    }

    let name_field = |value: &str, what: &str| {
        let value = value.trim();
        if value.is_empty() || value.chars().count() > MAX_NAME_LENGTH {
            Err(PurchaseError::InvalidInput(format!(
                "{what} is required (at most {MAX_NAME_LENGTH} characters)"
            )))
        } else {
            Ok(value.to_string())
        }
    };

    let address_field = |value: &Option<String>, what: &str| {
        optional_field(value.as_ref(), MAX_ADDRESS_FIELD_LENGTH, what)
    };
    let address = Address {
        line1: address_field(&input.address.line1, "The address")?,
        line2: address_field(&input.address.line2, "The address")?,
        city: address_field(&input.address.city, "The city")?,
        country: Some(country),
        state: address_field(&input.address.state, "The state")?,
        zip_code: address_field(&input.address.zip_code, "The postal code")?,
    };
    let vat_number = optional_field(input.vat_number.as_ref(), MAX_TAX_ID_LENGTH, "The tax ID")?;
    let legal_number = optional_field(
        input.legal_number.as_ref(),
        MAX_TAX_ID_LENGTH,
        "The registration number",
    )?;

    let (customer_type, name, first_name, last_name) = match &input.buyer {
        Buyer::Company { name } => (
            CustomerType::Company,
            name_field(name, "The company name")?,
            None,
            None,
        ),
        Buyer::Individual {
            first_name,
            last_name,
        } => {
            if vat_number.is_some() || legal_number.is_some() {
                return Err(invalid(
                    "Tax IDs and registration numbers are for companies only",
                ));
            }
            let first = name_field(first_name, "The first name")?;
            let last = name_field(last_name, "The last name")?;
            (
                CustomerType::Individual,
                format!("{first} {last}"),
                Some(first),
                Some(last),
            )
        }
    };

    Ok(ValidatedBuyer {
        email,
        customer_type,
        name,
        first_name,
        last_name,
        address,
        vat_number,
        legal_number,
    })
}

/// What the tenant configured on the link and its invoicing entity, which the page only reflects.
fn check_link_options(
    link: &PurchaseLink,
    buyer: &ValidatedBuyer,
    requires_billing_information: bool,
) -> Result<(), PurchaseError> {
    let invalid = |msg: &str| Err(PurchaseError::InvalidInput(msg.to_string()));
    if !link.allowed_buyer_types.allows(buyer.customer_type) {
        return match link.allowed_buyer_types {
            AllowedBuyerTypes::IndividualsOnly => {
                invalid("This plan can only be purchased by individuals")
            }
            _ => invalid("This plan can only be purchased by companies"),
        };
    }
    check_billing_address(
        buyer,
        link.require_billing_address,
        requires_billing_information,
    )?;
    match (
        link.tax_id_collection,
        buyer.customer_type,
        &buyer.vat_number,
    ) {
        (TaxIdCollection::Off, _, Some(_)) => {
            return invalid("Tax IDs are not collected for this purchase");
        }
        (TaxIdCollection::Required, CustomerType::Company, None) => {
            return invalid("A tax ID is required for companies");
        }
        _ => {}
    }
    Ok(())
}

/// The link may ask for a street and city; an invoicing entity requiring billing information asks
/// for the complete address up front, which its checkout would refuse to complete without.
fn check_billing_address(
    buyer: &ValidatedBuyer,
    link_requires_address: bool,
    requires_billing_information: bool,
) -> Result<(), PurchaseError> {
    if requires_billing_information
        && !is_complete_billing_information(Some(&buyer.email), Some(&buyer.address))
    {
        return Err(PurchaseError::InvalidInput(
            "A complete billing address (street, postal code and city) is required".to_string(),
        ));
    }
    if link_requires_address && (buyer.address.line1.is_none() || buyer.address.city.is_none()) {
        return Err(PurchaseError::InvalidInput(
            "A billing address (street and city) is required".to_string(),
        ));
    }
    Ok(())
}

/// The plan option a purchase buys, and the currency it is priced in.
struct SoldOption {
    pricing: OptionPricing,
    option_alias: String,
    option_name: String,
}

/// A link sells its option (else the plan's default) in the plan's currency, else the first one
/// the option is sold in. An option the plan no longer has is the seller's to fix.
fn sold_option(
    link: &LivePurchaseLink,
    options: &[PlanOption],
    components: &[PriceComponent],
) -> Result<SoldOption, PurchaseError> {
    let asked = link.link.option.clone().map(PlanOptionRef::Alias);
    let option = resolve_option(options, asked.as_ref()).map_err(|e| {
        log::warn!("Purchase link {} sells no option: {e:?}", link.link.id);
        PurchaseError::Unavailable
    })?;
    let sells = |currency: &String| OptionPricing::new(option.id, currency).sells(components);
    let currency = std::iter::once(link.version.currency.clone())
        .chain(currencies(components))
        .find(sells)
        .unwrap_or_else(|| link.version.currency.clone());
    Ok(SoldOption {
        pricing: OptionPricing::new(option.id, &currency),
        option_alias: option.alias.clone(),
        option_name: option.name.clone(),
    })
}

/// The level each usage-levels charge of the option sold starts on: the buyer's pick, else its
/// lowest (`usage_levels::starting_levels`). A level the plan does not sell is the buyer's to fix.
fn purchase_levels(
    sold: &SoldOption,
    mut components: Vec<PriceComponent>,
    chosen: &[ChosenLevel],
) -> Result<Vec<ComponentParameterization>, PurchaseError> {
    sold.pricing.narrow(&mut components);
    usage_levels::starting_levels(&components, chosen).map_err(|e| match e {
        StoreError::InvalidArgument(why) => PurchaseError::InvalidInput(why),
        other => PurchaseError::Internal(Report::new(other)),
    })
}

/// Whether a checkout bills the levels a purchase asks.
fn same_levels(
    checkout: &[ComponentParameterization],
    asked: &[ComponentParameterization],
) -> bool {
    let key = |params: &[ComponentParameterization]| {
        let mut key: Vec<_> = params
            .iter()
            .map(|p| (p.component_id, p.parameters.committed_capacity))
            .collect();
        key.sort_by_key(|(id, _)| *id);
        key
    };
    key(checkout) == key(asked)
}

fn new_customer(buyer: &ValidatedBuyer, currency: &str) -> CustomerNew {
    CustomerNew {
        name: buyer.name.clone(),
        alias: None,
        billing_email: Some(buyer.email.clone()),
        invoicing_emails: vec![buyer.email.clone()],
        phone: None,
        currency: currency.to_string(),
        billing_address: Some(buyer.address.clone()),
        shipping_address: None,
        invoicing_entity_id: None,
        force_created_date: None,
        vat_number: buyer.vat_number.clone(),
        legal_number: buyer.legal_number.clone(),
        buyer_reference: None,
        custom_taxes: vec![],
        tax_status: CustomerTaxStatus::Taxable,
        exemption_reason: None,
        connected_account_id: None,
        custom_properties: serde_json::json!({}),
        preferred_locales: vec![],
        customer_type: buyer.customer_type,
        first_name: buyer.first_name.clone(),
        last_name: buyer.last_name.clone(),
    }
}

/// What a resubmission may change: the buyer's identity, contact and billing details.
fn customer_patch(id: CustomerId, buyer: &ValidatedBuyer) -> CustomerPatch {
    CustomerPatch {
        id,
        name: Some(buyer.name.clone()),
        alias: None,
        billing_email: Some(buyer.email.clone()),
        invoicing_emails: Some(vec![buyer.email.clone()]),
        phone: None,
        currency: None,
        billing_address: Some(buyer.address.clone()),
        shipping_address: None,
        invoicing_entity_id: None,
        vat_number: Some(buyer.vat_number.clone()),
        legal_number: Some(buyer.legal_number.clone()),
        buyer_reference: None,
        custom_taxes: None,
        current_payment_method_id: None,
        tax_status: None,
        exemption_reason: None,
        connected_account_id: None,
        custom_properties: None,
        preferred_locales: None,
        customer_type: Some(buyer.customer_type),
        first_name: buyer.first_name.clone(),
        last_name: buyer.last_name.clone(),
    }
}

/// Tenant-side setup problems (currency not enabled, required custom properties) are not
/// the buyer's to fix, and their details are not the buyer's to read.
fn refused_or_internal(e: Report<StoreError>) -> PurchaseError {
    match e.current_context() {
        StoreError::InvalidArgument(_) | StoreError::ValueNotFound(_) => {
            log::warn!("Purchase refused while creating the customer: {e:?}");
            PurchaseError::Unavailable
        }
        _ => PurchaseError::Internal(e),
    }
}

/// `(instance_id, instance_public_key)` of a valid request signed by an installation.
fn verify_license_request(request: &str) -> Result<(InstanceId, String), PurchaseError> {
    let invalid = || {
        PurchaseError::InvalidInput(
            "The request from your installation is invalid or expired, start again from it"
                .to_string(),
        )
    };
    if request.len() > MAX_LICENSE_REQUEST_LENGTH {
        return Err(invalid());
    }
    let verified = PurchaseRequest::verify(request, common_clock::wall().timestamp())
        .map_err(|_| invalid())?;
    let instance_id = InstanceId::parse(&verified.instance_id).map_err(|_| invalid())?;
    Ok((instance_id, verified.instance_public_key))
}

fn hourly_bucket(name: &str, per_hour: u32) -> RateLimitConfig {
    let per_hour = f64::from(per_hour.max(1));
    RateLimitConfig {
        capacity: per_hour,
        refill_rate: per_hour / 3600.0,
        window_seconds: 3600,
        bucket: Some(name.to_string()),
    }
}

/// A checkout another coupon code would cancel: only one no payment was started on.
fn refuse_superseding(status: CheckoutSessionStatus) -> Result<(), PurchaseError> {
    if status == CheckoutSessionStatus::Created {
        return Ok(());
    }
    Err(PurchaseError::InvalidInput(
        "A payment is in progress for this purchase".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(email: &str, buyer: Buyer, country: &str) -> StartPurchase {
        StartPurchase {
            token: "t".to_string(),
            email: email.to_string(),
            buyer,
            country: country.to_string(),
            address: BillingAddressInput::default(),
            vat_number: None,
            legal_number: None,
            pow_solution: String::new(),
            attempt_id: "0f3c2a1e-6b7d-4e2f-9a8b-1c2d3e4f5a6b".to_string(),
            license_request: None,
            client_ip: None,
            coupon_code: None,
            levels: vec![],
        }
    }

    #[test]
    fn a_new_coupon_code_cancels_only_a_checkout_without_a_payment() {
        assert!(refuse_superseding(CheckoutSessionStatus::Created).is_ok());
        match refuse_superseding(CheckoutSessionStatus::AwaitingPayment) {
            Err(PurchaseError::InvalidInput(message)) => {
                assert_eq!(message, "A payment is in progress for this purchase");
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn buyer_input_is_validated() {
        let company = || Buyer::Company {
            name: "Acme".to_string(),
        };
        assert!(validate(&input("buyer@acme.com", company(), "fr")).is_ok());
        assert!(validate(&input("not-an-email", company(), "FR")).is_err());
        assert!(validate(&input("buyer@acme.com", company(), "XX")).is_err());
        let long = format!("{}@acme.com", "a".repeat(250));
        assert!(validate(&input(&long, company(), "FR")).is_err());
        let mut bad_attempt = input("buyer@acme.com", company(), "FR");
        bad_attempt.attempt_id = "short".to_string();
        assert!(validate(&bad_attempt).is_err());
        assert!(
            validate(&input(
                "buyer@acme.com",
                Buyer::Individual {
                    first_name: "Ada".to_string(),
                    last_name: " ".to_string(),
                },
                "FR"
            ))
            .is_err()
        );

        let mut blank = input("buyer@acme.com", company(), "FR");
        blank.vat_number = Some("  ".to_string());
        blank.address.city = Some(" Paris ".to_string());
        let validated = validate(&blank).unwrap();
        assert_eq!(validated.vat_number, None);
        assert_eq!(validated.address.city.as_deref(), Some("Paris"));

        let mut individual_vat = input(
            "buyer@acme.com",
            Buyer::Individual {
                first_name: "Ada".to_string(),
                last_name: "Lovelace".to_string(),
            },
            "FR",
        );
        individual_vat.vat_number = Some("FR12345678901".to_string());
        assert!(validate(&individual_vat).is_err());
    }

    #[test]
    fn billing_address_follows_the_link_and_the_invoicing_entity() {
        let company = || Buyer::Company {
            name: "Acme".to_string(),
        };
        let country_only = validate(&input("buyer@acme.com", company(), "FR")).unwrap();
        assert!(check_billing_address(&country_only, false, false).is_ok());
        assert!(check_billing_address(&country_only, true, false).is_err());
        assert!(check_billing_address(&country_only, false, true).is_err());

        let mut street = input("buyer@acme.com", company(), "FR");
        street.address.line1 = Some("1 Rue de la Paix".to_string());
        street.address.city = Some("Paris".to_string());
        let street_and_city = validate(&street).unwrap();
        assert!(check_billing_address(&street_and_city, true, false).is_ok());
        assert!(check_billing_address(&street_and_city, true, true).is_err());

        street.address.zip_code = Some("75002".to_string());
        let complete = validate(&street).unwrap();
        assert!(check_billing_address(&complete, true, true).is_ok());
        assert!(check_billing_address(&complete, false, true).is_ok());
    }
}
