//! The trial emails (R-158): queued where the trial starts, is noticed and ends; what follows the
//! trial, the payment method and the amount are read when the email is built.
use super::{Build, SkipReason};
use crate::StoreResult;
use crate::domain::SubscriptionDetails;
use crate::domain::customer_emails::{EmailContext, EmailKind, TrialEndedOutcome};
use crate::domain::customer_payment_methods::ResolvedPaymentMethod;
use crate::domain::enums::BillingPeriodEnum;
use crate::domain::trials::{TrialEnd, TrialOutcome};
use crate::errors::StoreError;
use crate::repositories::customer_payment_methods::CustomerPaymentMethodsInterface;
use crate::repositories::subscriptions::SubscriptionInterfaceAuto;
use crate::services::Services;
use crate::store::PgConn;
use chrono::NaiveDate;
use common_domain::ids::{BaseId, CustomerId, PlanVersionId, SubscriptionId, TenantId};
use diesel_models::enums::SubscriptionStatusEnum;
use diesel_models::plans::PlanRow;
use diesel_models::subscription_facts::SubscriptionTrialRow;
use diesel_models::subscriptions::SubscriptionRow;
use diesel_models::trial_offers::TrialOfferRow;
use error_stack::Report;
use meteroid_mailer::render::{
    BillingInterval, Body, Cta, TrialCharge, TrialFollowUp, TrialWithoutCard,
};

fn db<T>(result: diesel_models::DbResult<T>) -> StoreResult<T> {
    result.map_err(Into::<Report<StoreError>>::into)
}

fn is_manual(trial: &SubscriptionTrialRow) -> bool {
    TrialEnd::parse(&trial.end_action) == TrialEnd::Manual
}

/// The subscription's trial, unless its end goes to the operator (a manual end): the customer is
/// not emailed about it.
async fn customer_facing_trial(
    conn: &mut PgConn,
    subscription_id: SubscriptionId,
) -> StoreResult<Option<SubscriptionTrialRow>> {
    Ok(db(SubscriptionTrialRow::find(conn, subscription_id).await)?.filter(|t| !is_manual(t)))
}

/// Queues the trial-ending email with the notice of the trial ending on `ends_on`.
pub(crate) async fn enqueue_trial_ending_email_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
    subscription_id: SubscriptionId,
    ends_on: NaiveDate,
) -> StoreResult<()> {
    if customer_facing_trial(conn, subscription_id)
        .await?
        .is_none()
    {
        return Ok(());
    }
    let kind = EmailKind::TrialEnding;
    crate::repositories::customer_emails::enqueue_customer_email_on_tx(
        conn,
        tenant_id,
        kind,
        EmailContext::TrialEnding {
            customer_id,
            subscription_id,
            ends_on,
        },
        format!("{kind}:{subscription_id}:{ends_on}"),
    )
    .await
}

/// Queues the trial-ended email once the trial's `outcome` is recorded; a converted trial, or one
/// waiting for the operator's decision, sends none.
pub(crate) async fn enqueue_trial_ended_email_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
    subscription_id: SubscriptionId,
    ended_on: NaiveDate,
    outcome: TrialOutcome,
) -> StoreResult<()> {
    let outcome = match outcome {
        TrialOutcome::Cancelled => TrialEndedOutcome::Cancelled,
        TrialOutcome::Switched => TrialEndedOutcome::Switched,
        TrialOutcome::AwaitingPayment => TrialEndedOutcome::AwaitingPayment,
        TrialOutcome::Converted | TrialOutcome::AwaitingDecision => return Ok(()),
    };
    if customer_facing_trial(conn, subscription_id)
        .await?
        .is_none()
    {
        return Ok(());
    }
    let kind = EmailKind::TrialEnded;
    crate::repositories::customer_emails::enqueue_customer_email_on_tx(
        conn,
        tenant_id,
        kind,
        EmailContext::TrialEnded {
            customer_id,
            subscription_id,
            ended_on,
            outcome,
        },
        format!("{kind}:{subscription_id}:{ended_on}"),
    )
    .await
}

/// Queues the enrollment confirmation of a free trial that converts into automatic charges, once
/// the subscription is in it; the payment method on file is checked when the email is built.
pub(crate) async fn enqueue_trial_started_email_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
) -> StoreResult<()> {
    let subscription =
        db(SubscriptionRow::get_subscription_by_id(conn, &tenant_id, subscription_id).await)?
            .subscription;
    if subscription.status != SubscriptionStatusEnum::TrialActive
        || subscription.imported_at.is_some()
        || !subscription.charge_automatically
    {
        return Ok(());
    }
    let Some(trial) = customer_facing_trial(conn, subscription_id).await? else {
        return Ok(());
    };
    if !trial.free
        || TrialEnd::parse(&trial.end_action) != TrialEnd::Convert
        || trial.until_date <= common_clock::now().date_naive()
    {
        return Ok(());
    }
    let kind = EmailKind::TrialStarted;
    crate::repositories::customer_emails::enqueue_customer_email_on_tx(
        conn,
        tenant_id,
        kind,
        EmailContext::TrialStarted {
            customer_id: subscription.customer_id,
            subscription_id,
            starts_on: trial.from_date,
            ends_on: trial.until_date,
        },
        format!("{kind}:{subscription_id}:{}", trial.from_date),
    )
    .await
}

/// How the subscription pays once its trial ends.
enum TrialPayment {
    Invoiced,
    Card { method: Option<String> },
    Missing,
}

impl From<BillingPeriodEnum> for BillingInterval {
    fn from(period: BillingPeriodEnum) -> Self {
        match period {
            BillingPeriodEnum::Monthly => Self::Month,
            BillingPeriodEnum::Quarterly => Self::Quarter,
            BillingPeriodEnum::Semiannual => Self::HalfYear,
            BillingPeriodEnum::Annual => Self::Year,
        }
    }
}

fn subscription_of(context: &EmailContext) -> StoreResult<SubscriptionId> {
    match context {
        EmailContext::TrialEnding {
            subscription_id, ..
        }
        | EmailContext::TrialStarted {
            subscription_id, ..
        }
        | EmailContext::TrialEnded {
            subscription_id, ..
        } => Ok(*subscription_id),
        _ => Err(Report::new(StoreError::InvalidArgument(
            "not a trial email context".to_string(),
        ))),
    }
}

impl Services {
    /// The body of a trial email, its link left blank until [`Self::with_trial_link`]; a send
    /// skips what no longer applies (no payment method to confirm a trial start on).
    pub(super) async fn trial_email_body(
        &self,
        tenant_id: TenantId,
        kind: EmailKind,
        context: &EmailContext,
        build: Build,
    ) -> StoreResult<Result<Body, SkipReason>> {
        let subscription_id = subscription_of(context)?;
        let details = self
            .store
            .get_subscription_details(tenant_id, subscription_id)
            .await?;
        let subscription = &details.subscription;
        let plan_name = subscription.plan_name.clone();
        let blank = Cta::new(String::new());
        let mut conn = self.store.get_conn().await?;
        let trial = db(SubscriptionTrialRow::find(&mut conn, subscription_id).await)?;
        drop(conn);
        let manual = trial.as_ref().is_some_and(is_manual);
        if manual && build != Build::Preview {
            return Ok(Err(SkipReason::NotApplicable));
        }
        let end = trial.as_ref().map_or(TrialEnd::Convert, |trial| {
            TrialEnd::parse(&trial.end_action)
        });

        Ok(Ok(match (kind, context) {
            (EmailKind::TrialEnding, EmailContext::TrialEnding { ends_on, .. }) => {
                let offer = self.trial_offer_of(tenant_id, trial.as_ref()).await?;
                let follow_up = match end {
                    TrialEnd::Cancel => TrialFollowUp::Ends,
                    TrialEnd::Fallback => TrialFollowUp::Switches {
                        plan_name: self
                            .plan_name_of(
                                tenant_id,
                                offer.as_ref().and_then(|o| o.fallback_plan_version_id),
                            )
                            .await?,
                    },
                    TrialEnd::Convert | TrialEnd::Manual => {
                        match self.trial_payment(tenant_id, &details).await? {
                            TrialPayment::Invoiced => TrialFollowUp::Invoiced {
                                charge: self.trial_first_charge(&details).await,
                            },
                            TrialPayment::Card { method } => TrialFollowUp::Charged {
                                charge: self.trial_first_charge(&details).await,
                                method,
                            },
                            TrialPayment::Missing => TrialFollowUp::NoPaymentMethod(
                                self.without_card(tenant_id, trial.as_ref()).await?,
                            ),
                        }
                    }
                };
                Body::TrialEnding {
                    plan_name,
                    ends_on: *ends_on,
                    follow_up,
                    cta: blank,
                }
            }
            (
                EmailKind::TrialStarted,
                EmailContext::TrialStarted {
                    starts_on, ends_on, ..
                },
            ) => {
                if build != Build::Preview {
                    if end != TrialEnd::Convert || !subscription.charge_automatically {
                        return Ok(Err(SkipReason::NotApplicable));
                    }
                    if !matches!(
                        self.trial_payment(tenant_id, &details).await?,
                        TrialPayment::Card { .. }
                    ) {
                        return Ok(Err(SkipReason::NoPaymentMethod));
                    }
                }
                Body::TrialStarted {
                    plan_name,
                    starts_on: *starts_on,
                    ends_on: *ends_on,
                    after_trial: self.trial_first_charge(&details).await,
                    interval: subscription.period.into(),
                    cta: blank,
                }
            }
            (
                EmailKind::TrialEnded,
                EmailContext::TrialEnded {
                    ended_on, outcome, ..
                },
            ) => Body::TrialEnded {
                plan_name,
                ended_on: *ended_on,
                outcome: (*outcome).into(),
                cta: blank,
            },
            (kind, _) => {
                return Err(Report::new(StoreError::InvalidArgument(format!(
                    "{kind} cannot be rendered from a trial context"
                ))));
            }
        }))
    }

    /// The trial body with its portal link, minted once the email is sure to go out.
    pub(super) async fn with_trial_link(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        context: &EmailContext,
        body: Body,
        build: Build,
    ) -> StoreResult<Body> {
        let subscription_id = subscription_of(context)?;
        let today = common_clock::now().date_naive();
        Ok(match body {
            Body::TrialEnding {
                plan_name,
                ends_on,
                follow_up,
                ..
            } => {
                let url = match &follow_up {
                    TrialFollowUp::NoPaymentMethod(_) if build == Build::Preview => {
                        self.preview_card_update_link()
                    }
                    TrialFollowUp::NoPaymentMethod(_) => {
                        self.card_update_link(tenant_id, customer_id).await?
                    }
                    TrialFollowUp::Ends => {
                        self.subscriptions_link(tenant_id, customer_id, None, ends_on, build)
                            .await?
                    }
                    _ => {
                        self.subscriptions_link(
                            tenant_id,
                            customer_id,
                            Some(subscription_id),
                            ends_on,
                            build,
                        )
                        .await?
                    }
                };
                Body::TrialEnding {
                    plan_name,
                    ends_on,
                    follow_up,
                    cta: Cta::new(url),
                }
            }
            Body::TrialStarted {
                plan_name,
                starts_on,
                ends_on,
                after_trial,
                interval,
                ..
            } => Body::TrialStarted {
                cta: Cta::new(
                    self.subscriptions_link(
                        tenant_id,
                        customer_id,
                        Some(subscription_id),
                        ends_on,
                        build,
                    )
                    .await?,
                ),
                plan_name,
                starts_on,
                ends_on,
                after_trial,
                interval,
            },
            Body::TrialEnded {
                plan_name,
                ended_on,
                outcome,
                ..
            } => {
                let subscription = (outcome
                    == meteroid_mailer::render::TrialEndedOutcome::AwaitingPayment)
                    .then_some(subscription_id);
                let valid_until = today + chrono::Days::new(14);
                Body::TrialEnded {
                    cta: Cta::new(
                        self.subscriptions_link(
                            tenant_id,
                            customer_id,
                            subscription,
                            valid_until,
                            build,
                        )
                        .await?,
                    ),
                    plan_name,
                    ended_on,
                    outcome,
                }
            }
            body => body,
        })
    }

    /// The portal's subscriptions (one of them when given) on a token that may manage them, valid
    /// until `valid_until`, 3 to 30 days from now.
    async fn subscriptions_link(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        subscription_id: Option<SubscriptionId>,
        valid_until: NaiveDate,
        build: Build,
    ) -> StoreResult<String> {
        use crate::repositories::portal_tokens::PortalTokensInterface;
        let base = self.store.settings.public_url.trim_end_matches('/');
        let page = match subscription_id {
            Some(id) => format!("page=subscriptions&sub={}", id.as_base62()),
            None => "page=subscriptions".to_string(),
        };
        if build == Build::Preview {
            return Ok(format!("{base}/portal/customer?token=preview&{page}"));
        }
        let ttl = (valid_until - common_clock::now().date_naive())
            .clamp(chrono::Duration::days(3), chrono::Duration::days(30));
        let issued = self
            .store
            .customer_portal_token(
                tenant_id,
                customer_id,
                ttl,
                Some(vec![common_domain::auth::PortalScope::Manage]),
                false,
                crate::jwt_claims::ClientClaims::default(),
            )
            .await?;
        Ok(format!(
            "{base}/portal/customer?token={}&{page}",
            issued.token
        ))
    }

    /// As the trial's end reads it: invoiced when not charged automatically or paid by invoice
    /// (bank transfer, external), `Missing` when charged automatically with nothing to charge.
    async fn trial_payment(
        &self,
        tenant_id: TenantId,
        details: &SubscriptionDetails,
    ) -> StoreResult<TrialPayment> {
        let subscription = &details.subscription;
        if !subscription.charge_automatically {
            return Ok(TrialPayment::Invoiced);
        }
        let mut conn = self.store.get_conn().await?;
        let lacks = self
            .store
            .lacks_payment_method_tx(&mut conn, tenant_id, subscription.id, true)
            .await?;
        drop(conn);
        if lacks {
            return Ok(TrialPayment::Missing);
        }
        Ok(
            match self
                .store
                .resolve_payment_method_for_subscription(tenant_id, subscription.id)
                .await?
            {
                ResolvedPaymentMethod::CustomerPaymentMethod(id) => {
                    let method = self.store.get_payment_method_by_id(&tenant_id, &id).await?;
                    TrialPayment::Card {
                        method: method.card_last4.is_some().then(|| {
                            crate::services::payment_reminders::card_expiry::card_label(
                                method.card_brand.as_deref(),
                                method.card_last4.as_deref(),
                            )
                        }),
                    }
                }
                ResolvedPaymentMethod::BankTransfer(_) | ResolvedPaymentMethod::NotConfigured => {
                    TrialPayment::Invoiced
                }
            },
        )
    }

    /// The first invoice after the trial as the upcoming invoice estimates it; none when it bills
    /// nothing or cannot be estimated, so the email states no amount rather than a wrong one.
    async fn trial_first_charge(&self, details: &SubscriptionDetails) -> Option<TrialCharge> {
        let estimate = async {
            let mut conn = self.store.get_conn().await?;
            self.compute_upcoming_invoice(&mut conn, details).await
        }
        .await;
        match estimate {
            Ok(content) if content.amount_due > 0 => Some(TrialCharge {
                amount_minor: content.amount_due,
                currency: details.subscription.currency.clone(),
            }),
            Ok(_) => None,
            Err(e) => {
                log::warn!(
                    "Trial first charge of subscription {} not estimated: {e:?}",
                    details.subscription.id
                );
                None
            }
        }
    }

    async fn trial_offer_of(
        &self,
        tenant_id: TenantId,
        trial: Option<&SubscriptionTrialRow>,
    ) -> StoreResult<Option<TrialOfferRow>> {
        let Some(offer_id) = trial.and_then(|trial| trial.trial_offer_id) else {
            return Ok(None);
        };
        let mut conn = self.store.get_conn().await?;
        db(TrialOfferRow::find(&mut conn, &tenant_id, offer_id).await)
    }

    /// What the trial's end does without a payment method, as given at its start.
    async fn without_card(
        &self,
        tenant_id: TenantId,
        trial: Option<&SubscriptionTrialRow>,
    ) -> StoreResult<TrialWithoutCard> {
        let recorded = trial.and_then(|trial| {
            crate::domain::trials::TrialWithoutCard::parse(
                &trial.without_card,
                trial.without_card_plan_version_id,
            )
        });
        Ok(match recorded.unwrap_or_default() {
            crate::domain::trials::TrialWithoutCard::AwaitPayment => TrialWithoutCard::AwaitPayment,
            crate::domain::trials::TrialWithoutCard::Cancel => TrialWithoutCard::Cancel,
            crate::domain::trials::TrialWithoutCard::Switch { plan_version_id } => {
                TrialWithoutCard::Switch {
                    plan_name: self.plan_name_of(tenant_id, Some(plan_version_id)).await?,
                }
            }
        })
    }

    async fn plan_name_of(
        &self,
        tenant_id: TenantId,
        plan_version_id: Option<PlanVersionId>,
    ) -> StoreResult<String> {
        let plan_version_id = plan_version_id.ok_or_else(|| {
            Report::new(StoreError::InvalidArgument(
                "the trial switches to a plan it does not name".to_string(),
            ))
        })?;
        let mut conn = self.store.get_conn().await?;
        Ok(db(
            PlanRow::get_with_version_by_version_id(&mut conn, plan_version_id, tenant_id).await,
        )?
        .plan
        .name)
    }
}
