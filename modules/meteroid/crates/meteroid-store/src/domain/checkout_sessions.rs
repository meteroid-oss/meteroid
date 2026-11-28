use chrono::{DateTime, Duration, NaiveDate, Utc};
use common_domain::ids::{
    BaseId, CheckoutSessionId, CouponId, CustomerId, PlanVersionId, ResourceId, SubscriptionId,
    TenantId,
};
use diesel_models::checkout_sessions::{CheckoutSessionRow, CheckoutSessionRowNew};
use diesel_models::enums::{CheckoutSessionStatusEnum, CheckoutTypeEnum};
use o2o::o2o;
use rust_decimal::Decimal;

use crate::domain::subscription_add_ons::CreateSubscriptionAddOns;
use crate::domain::subscription_components::CreateSubscriptionComponents;
use crate::domain::subscriptions::{CreateSubscription, PaymentMethodsConfig, SubscriptionNew};
use crate::domain::{
    CreateSubscriptionCoupon, CreateSubscriptionCoupons, SubscriptionActivationCondition,
};

#[derive(Debug, Clone, PartialEq, Eq, o2o)]
#[map_owned(CheckoutSessionStatusEnum)]
pub enum CheckoutSessionStatus {
    Created,
    AwaitingPayment,
    Completed,
    Expired,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, o2o)]
#[map_owned(CheckoutTypeEnum)]
pub enum CheckoutType {
    #[default]
    SelfServe,
    SubscriptionActivation,
    PlanChange,
    AddonPurchase,
    TopUp,
}

#[derive(Debug, Clone)]
pub struct CheckoutSession {
    pub id: CheckoutSessionId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub plan_version_id: Option<PlanVersionId>,
    /// The plan option bought, by alias; none: the plan's default in the customer's currency.
    pub option: Option<String>,

    // Basic subscription parameters
    pub billing_start_date: Option<NaiveDate>,
    pub billing_day_anchor: Option<i16>,
    pub net_terms: Option<i32>,
    /// The trial it asks, as it was sent (R-153).
    pub trial: crate::domain::trials::TrialRequest,
    /// The offer it asks, as it was sent (R-170).
    pub offer: crate::domain::offers::OfferRequest,
    /// The trial in days as v1 reads it: none the plan's, 0 none.
    pub trial_duration_days: Option<i32>,
    pub end_date: Option<NaiveDate>,

    // Billing options
    pub auto_advance_invoices: bool,
    pub charge_automatically: bool,
    pub invoice_memo: Option<String>,
    pub invoice_threshold: Option<Decimal>,
    pub purchase_order: Option<String>,

    // Payment configuration
    pub payment_methods_config: Option<PaymentMethodsConfig>,

    // Complex parameters
    pub components: Option<CreateSubscriptionComponents>,
    pub add_ons: Option<CreateSubscriptionAddOns>,

    // Coupons
    pub coupon_code: Option<String>,
    pub coupon_ids: Vec<CouponId>,

    // Session state
    pub status: CheckoutSessionStatus,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub subscription_id: Option<SubscriptionId>,
    pub metadata: Option<serde_json::Value>,
    pub checkout_type: CheckoutType,
    pub change_date: Option<NaiveDate>,
    pub success_url: Option<String>,
    pub cancel_url: Option<String>,
    /// The stored resource the subscription is for (§3.5).
    pub resource_id: Option<ResourceId>,
    /// On creation, what its subscription would be accepted with: `plan_sold_for_mismatch`,
    /// `metric_not_attributed`.
    pub warnings: Vec<String>,
    /// With `metric_not_attributed`: the codes of the metrics its resource would never bill.
    pub metrics_not_attributed: Vec<crate::domain::usage_attribution::MetricCodes>,
}

impl CheckoutSession {
    pub fn subscription_plan_version_id(&self) -> crate::StoreResult<PlanVersionId> {
        self.plan_version_id.ok_or_else(|| {
            crate::errors::StoreError::InvalidArgument(
                "Top-up checkout has no subscription plan".into(),
            )
            .into()
        })
    }

    /// What a plan-change session moves to: its option of its plan version.
    pub fn plan_target(
        &self,
    ) -> crate::StoreResult<crate::domain::subscription_changes::PlanTarget> {
        Ok(crate::domain::subscription_changes::PlanTarget {
            version: self.subscription_plan_version_id()?,
            option: self.option_ref(),
            prices: Vec::new(),
        })
    }

    /// The parameters a plan change checkout moves with: the levels its usage-levels charges land
    /// on, as the customer picked them.
    pub fn component_params(
        &self,
    ) -> Vec<crate::domain::subscription_components::ComponentParameterization> {
        self.components
            .as_ref()
            .map(|c| c.parameterized_components.clone())
            .unwrap_or_default()
    }

    /// The option bought, as subscription creation and plan changes name it.
    pub fn option_ref(&self) -> Option<crate::domain::plan_options::PlanOptionRef> {
        self.option
            .clone()
            .map(crate::domain::plan_options::PlanOptionRef::Alias)
    }

    pub fn is_expired(&self) -> bool {
        self.status == CheckoutSessionStatus::Expired
            || self.expires_at.is_some_and(|exp| common_clock::now() > exp)
    }

    pub fn is_completed(&self) -> bool {
        self.status == CheckoutSessionStatus::Completed
    }

    pub fn can_complete(&self) -> bool {
        // Allow completion from Created (first attempt) or AwaitingPayment (retry after failed payment)
        (self.status == CheckoutSessionStatus::Created
            || self.status == CheckoutSessionStatus::AwaitingPayment)
            && !self.is_expired()
    }

    /// The trial the subscription it creates asks for (R-153).
    pub fn trial_request(&self) -> crate::domain::trials::TrialRequest {
        self.trial.clone()
    }

    /// The offer the subscription it creates asks for (R-170).
    pub fn offer_request(&self) -> crate::domain::offers::OfferRequest {
        self.offer
    }

    pub fn to_subscription_new(
        &self,
        start_date: NaiveDate,
    ) -> crate::StoreResult<SubscriptionNew> {
        Ok(SubscriptionNew {
            customer_id: self.customer_id,
            plan_version_id: self.subscription_plan_version_id()?,
            net_terms: self.net_terms.map(|n| n as u32),
            invoice_memo: self.invoice_memo.clone(),
            invoice_threshold: self.invoice_threshold,
            start_date,
            end_date: self.end_date,
            billing_start_date: self.billing_start_date,
            // Use OnCheckout activation for proper period setup, but skip_checkout_session
            // prevents creating another checkout session (we're already completing one)
            activation_condition: SubscriptionActivationCondition::OnCheckout,
            offer: self.offer_request(),
            trial: self.trial_request(),
            billing_day_anchor: self.billing_day_anchor.map(|a| a as u16),
            payment_methods_config: self.payment_methods_config.clone(),
            auto_advance_invoices: self.auto_advance_invoices,
            charge_automatically: self.charge_automatically,
            purchase_order: self.purchase_order.clone(),
            backdate_invoices: false,
            skip_checkout_session: true,
            skip_past_invoices: false,
            resource_id: self.resource_id,
            custom_properties: serde_json::Value::Null,
            external_id: None,
        })
    }

    pub fn to_create_subscription(
        &self,
        start_date: NaiveDate,
        coupon_ids: Vec<CouponId>,
    ) -> crate::StoreResult<CreateSubscription> {
        let coupons = if coupon_ids.is_empty() {
            None
        } else {
            Some(CreateSubscriptionCoupons {
                coupons: coupon_ids
                    .into_iter()
                    .map(|coupon_id| CreateSubscriptionCoupon { coupon_id })
                    .collect(),
            })
        };

        Ok(CreateSubscription {
            subscription: self.to_subscription_new(start_date)?,
            option: self.option_ref(),
            price_components: self.components.clone(),
            add_ons: self.add_ons.clone(),
            coupons,
            entitlements: vec![],
            minimum_override: None,
            drop_minimum: false,
        })
    }
}

impl From<CheckoutSessionRow> for CheckoutSession {
    fn from(row: CheckoutSessionRow) -> Self {
        let session_id = row.id;

        let components: Option<CreateSubscriptionComponents> = row.components.and_then(|v| {
            serde_json::from_value(v.clone()).map_err(|e| {
                log::error!(
                    "Failed to deserialize components for checkout session {}: {}. This may result in a subscription without the expected price components!",
                    session_id, e
                );
                e
            }).ok()
        });

        let add_ons: Option<CreateSubscriptionAddOns> = row.add_ons.and_then(|v| {
            serde_json::from_value(v.clone()).map_err(|e| {
                log::error!(
                    "Failed to deserialize add_ons for checkout session {}: {}. This may result in a subscription without the expected add-ons!",
                    session_id, e
                );
                e
            }).ok()
        });

        let payment_methods_config: Option<PaymentMethodsConfig> =
            row.payment_methods_config.and_then(|v| {
                serde_json::from_value(v.clone())
                    .map_err(|e| {
                        log::error!(
                        "Failed to deserialize payment_methods_config for checkout session {}: {}",
                        session_id, e
                    );
                        e
                    })
                    .ok()
            });

        let trial = stored_trial(
            session_id,
            row.trial_request,
            row.trial_duration_days,
            row.trial_free,
            row.trial_offer_id,
        );
        Self {
            id: row.id,
            tenant_id: row.tenant_id,
            customer_id: row.customer_id,
            plan_version_id: row.plan_version_id,
            option: row.plan_option,
            billing_start_date: row.billing_start_date,
            billing_day_anchor: row.billing_day_anchor,
            net_terms: row.net_terms,
            trial,
            offer: crate::domain::offers::OfferRequest::from_stored(session_id, row.offer_request),
            trial_duration_days: row.trial_duration_days,
            end_date: row.end_date,
            auto_advance_invoices: row.auto_advance_invoices,
            charge_automatically: row.charge_automatically,
            invoice_memo: row.invoice_memo,
            invoice_threshold: row.invoice_threshold,
            purchase_order: row.purchase_order,
            payment_methods_config,
            components,
            add_ons,
            coupon_code: row.coupon_code,
            coupon_ids: row.coupon_ids.into_iter().flatten().collect(),
            status: row.status.into(),
            created_at: row.created_at,
            expires_at: row.expires_at,
            completed_at: row.completed_at,
            subscription_id: row.subscription_id,
            metadata: row.metadata,
            checkout_type: row.checkout_type.into(),
            change_date: row.change_date,
            success_url: row.success_url,
            cancel_url: row.cancel_url,
            resource_id: row.resource_id,
            warnings: Vec::new(),
            metrics_not_attributed: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CreateCheckoutSession {
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub plan_version_id: PlanVersionId,
    /// The plan option bought, by alias, of those sold in the customer's currency.
    pub option: Option<String>,

    // Basic subscription parameters
    pub billing_start_date: Option<NaiveDate>,
    pub billing_day_anchor: Option<i16>,
    pub net_terms: Option<i32>,
    /// Days or an end date counted from the billing start, else today.
    pub trial: crate::domain::trials::TrialRequest,
    /// The offer the subscription it creates is given (R-170).
    pub offer: crate::domain::offers::OfferRequest,
    pub end_date: Option<NaiveDate>,

    // Billing options
    pub auto_advance_invoices: bool,
    pub charge_automatically: bool,
    pub invoice_memo: Option<String>,
    pub invoice_threshold: Option<Decimal>,
    pub purchase_order: Option<String>,

    // Payment configuration
    pub payment_methods_config: Option<PaymentMethodsConfig>,

    // Complex parameters
    pub components: Option<CreateSubscriptionComponents>,
    pub add_ons: Option<CreateSubscriptionAddOns>,

    // Coupons
    pub coupon_code: Option<String>,
    pub coupon_ids: Vec<CouponId>,

    // Session options
    pub expires_in_hours: Option<u32>,
    pub metadata: Option<serde_json::Value>,
    pub checkout_type: CheckoutType,
    pub subscription_id: Option<SubscriptionId>,
    pub change_date: Option<NaiveDate>,
    pub success_url: Option<String>,
    pub cancel_url: Option<String>,
    /// The stored resource the subscription is for (§3.5).
    pub resource_id: Option<ResourceId>,
}

impl CreateCheckoutSession {
    pub fn try_into_row(self) -> Result<CheckoutSessionRowNew, crate::errors::StoreError> {
        // Checked: a term past what a date holds is refused, never a panic.
        let expires_at = self
            .expires_in_hours
            .map(|h| {
                Duration::try_hours(i64::from(h))
                    .and_then(|term| common_clock::now().checked_add_signed(term))
                    .ok_or_else(|| {
                        crate::errors::StoreError::InvalidArgument(
                            "a checkout session's expiry is too far ahead".to_owned(),
                        )
                    })
            })
            .transpose()?;

        let components = self
            .components
            .map(serde_json::to_value)
            .transpose()
            .map_err(|e| crate::errors::StoreError::SerdeError("components".to_string(), e))?;
        let add_ons = self
            .add_ons
            .map(serde_json::to_value)
            .transpose()
            .map_err(|e| crate::errors::StoreError::SerdeError("add_ons".to_string(), e))?;
        let payment_methods_config = self
            .payment_methods_config
            .map(serde_json::to_value)
            .transpose()
            .map_err(|e| {
                crate::errors::StoreError::SerdeError("payment_methods_config".to_string(), e)
            })?;

        let (trial_duration_days, trial_free, trial_offer_id) = trial_columns(
            &self.trial,
            self.billing_start_date
                .unwrap_or_else(|| common_clock::now().date_naive()),
        );
        let trial_request = StoredTrial::of(&self.trial)
            .map(serde_json::to_value)
            .transpose()
            .map_err(|e| crate::errors::StoreError::SerdeError("trial_request".to_string(), e))?;
        Ok(CheckoutSessionRowNew {
            id: CheckoutSessionId::new(),
            tenant_id: self.tenant_id,
            customer_id: self.customer_id,
            plan_version_id: Some(self.plan_version_id),
            plan_option: self.option,
            billing_start_date: self.billing_start_date,
            billing_day_anchor: self.billing_day_anchor,
            net_terms: self.net_terms,
            trial_duration_days,
            end_date: self.end_date,
            auto_advance_invoices: self.auto_advance_invoices,
            charge_automatically: self.charge_automatically,
            invoice_memo: self.invoice_memo,
            invoice_threshold: self.invoice_threshold,
            purchase_order: self.purchase_order,
            payment_methods_config,
            components,
            add_ons,
            coupon_code: self.coupon_code,
            coupon_ids: self.coupon_ids.into_iter().map(Some).collect(),
            expires_at,
            metadata: self.metadata,
            checkout_type: self.checkout_type.into(),
            subscription_id: self.subscription_id,
            change_date: self.change_date,
            success_url: self.success_url,
            cancel_url: self.cancel_url,
            resource_id: self.resource_id,
            trial_free,
            trial_offer_id,
            trial_request,
            offer_request: self.offer.stored(),
        })
    }
}

/// A trial request as a session stores it: its days, freeness and offer.
fn trial_columns(
    trial: &crate::domain::trials::TrialRequest,
    billing_start: NaiveDate,
) -> (
    Option<i32>,
    Option<bool>,
    Option<common_domain::ids::TrialOfferId>,
) {
    use crate::domain::trials::{TrialLength, TrialRequest};
    match trial {
        TrialRequest::Default | TrialRequest::Plan => (None, None, None),
        TrialRequest::None => (Some(0), None, None),
        TrialRequest::Offer(offer) => (None, None, Some(*offer)),
        TrialRequest::Custom { length, free, .. } => {
            let days = match length {
                TrialLength::Days(days) => i64::from(*days),
                TrialLength::Until(until) => (*until - billing_start).num_days().max(0),
            };
            (Some(i32::try_from(days).unwrap_or(i32::MAX)), *free, None)
        }
    }
}

/// A trial request as a session stores it, read back as it was sent; `Default` is stored as none.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum StoredTrial {
    Plan,
    None,
    Offer {
        offer: common_domain::ids::TrialOfferId,
    },
    Custom {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        days: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        until: Option<NaiveDate>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        free: Option<bool>,
        #[serde(default)]
        features_of: StoredFeaturesOf,
        #[serde(default)]
        manual: bool,
    },
}

/// Whose features a custom trial gives.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum StoredFeaturesOf {
    /// Its plan's trial's.
    #[default]
    PlanTrial,
    /// The subscription's own plan's.
    Own,
    Plan {
        plan: common_domain::ids::PlanId,
    },
}

impl StoredTrial {
    fn of(trial: &crate::domain::trials::TrialRequest) -> Option<Self> {
        use crate::domain::trials::{TrialLength, TrialRequest};
        Some(match trial {
            TrialRequest::Default => return None,
            TrialRequest::Plan => StoredTrial::Plan,
            TrialRequest::None => StoredTrial::None,
            TrialRequest::Offer(offer) => StoredTrial::Offer { offer: *offer },
            TrialRequest::Custom {
                length,
                free,
                trial_plan,
                manual,
            } => {
                let (days, until) = match length {
                    TrialLength::Days(days) => (Some(*days), None),
                    TrialLength::Until(until) => (None, Some(*until)),
                };
                StoredTrial::Custom {
                    days,
                    until,
                    free: *free,
                    features_of: match trial_plan {
                        None => StoredFeaturesOf::PlanTrial,
                        Some(None) => StoredFeaturesOf::Own,
                        Some(Some(plan)) => StoredFeaturesOf::Plan { plan: *plan },
                    },
                    manual: *manual,
                }
            }
        })
    }

    fn request(self) -> Option<crate::domain::trials::TrialRequest> {
        use crate::domain::trials::{TrialLength, TrialRequest};
        Some(match self {
            StoredTrial::Plan => TrialRequest::Plan,
            StoredTrial::None => TrialRequest::None,
            StoredTrial::Offer { offer } => TrialRequest::Offer(offer),
            StoredTrial::Custom {
                days,
                until,
                free,
                features_of,
                manual,
            } => TrialRequest::Custom {
                length: match (days, until) {
                    (Some(days), None) => TrialLength::Days(days),
                    (None, Some(until)) => TrialLength::Until(until),
                    _ => return None,
                },
                free,
                trial_plan: match features_of {
                    StoredFeaturesOf::PlanTrial => None,
                    StoredFeaturesOf::Own => Some(None),
                    StoredFeaturesOf::Plan { plan } => Some(Some(plan)),
                },
                manual,
            },
        })
    }
}

/// The trial a session asks: as stored, else (a session created before) from its days, freeness
/// and offer.
fn stored_trial(
    session_id: CheckoutSessionId,
    stored: Option<serde_json::Value>,
    days: Option<i32>,
    free: Option<bool>,
    offer: Option<common_domain::ids::TrialOfferId>,
) -> crate::domain::trials::TrialRequest {
    use crate::domain::trials::TrialRequest;
    if let Some(value) = stored {
        match serde_json::from_value::<StoredTrial>(value)
            .ok()
            .and_then(StoredTrial::request)
        {
            Some(request) => return request,
            None => log::error!(
                "Checkout session {session_id} stores a trial request it cannot read; read from its days"
            ),
        }
    }
    match offer {
        Some(offer) => TrialRequest::Offer(offer),
        None => TrialRequest::from_days(days.map(|days| u32::try_from(days).unwrap_or(0)), free),
    }
}

#[derive(Debug, Clone)]
pub enum CheckoutCompletionResult {
    TopUpCompleted {
        transaction: crate::domain::PaymentTransaction,
    },
    /// Checkout completed successfully - subscription and optionally invoice created
    Completed {
        subscription_id: SubscriptionId,
        transaction: Option<crate::domain::PaymentTransaction>,
    },
    /// Payment is pending (async payment method like SEPA) - awaiting webhook confirmation
    /// No subscription or invoice created yet - will be created when payment settles
    AwaitingPayment {
        transaction: crate::domain::PaymentTransaction,
        /// Set when the customer must complete 3DS/SCA (transient client secret).
        next_action: Option<crate::domain::payment_transactions::PaymentNextAction>,
    },
}
