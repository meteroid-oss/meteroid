//! Trials (ADR 0015 §3): the plan's trial and the catalog of offers (`catalog`), the one resolution
//! of a creation's trial (`resolve`), what a customer is given once (`eligibility`), and a running
//! trial moved (`running`). Billing reads the subscription's trial fact, never an offer.
pub(crate) mod catalog;
pub(crate) mod eligibility;
pub(crate) mod ending;
pub(crate) mod resolve;
pub(crate) mod running;

use common_domain::actor::Actor;
use common_domain::ids::{AliasOr, PlanId, SubscriptionId, TenantId, TrialOfferId};

use crate::StoreResult;
use crate::domain::trials::{
    PlanTrial, SubscriptionTrial, TrialMove, TrialOffer, TrialOfferNew, TrialOfferPatch,
};
use crate::services::ServicesEdge;

pub(crate) use resolve::{PlanRef, resolve_trial_tx};
pub(crate) use running::trial_of_tx;

fn db<T>(result: diesel_models::DbResult<T>) -> StoreResult<T> {
    result.map_err(Into::into)
}

fn invalid(message: impl Into<String>) -> error_stack::Report<crate::errors::StoreError> {
    error_stack::Report::new(crate::errors::StoreError::InvalidArgument(message.into()))
}

impl crate::Store {
    /// Whether `terms` written as `plan_id`'s trial switch, at its end or without a card, to a
    /// paid plan's version they do not switch to already: Core switches to a free plan only.
    pub async fn trial_switches_to_paid(
        &self,
        tenant_id: TenantId,
        plan_id: PlanId,
        terms: &crate::domain::trials::TrialTerms,
    ) -> StoreResult<bool> {
        use crate::domain::trials::{TrialEndAction, TrialWithoutCard};
        let mut conn = self.get_conn().await?;
        let current = catalog::plan_trials_tx(&mut conn, tenant_id, &[plan_id])
            .await?
            .remove(&plan_id)
            .map(|trial| trial.offer);
        let at_end = match terms.end_action {
            Some(TrialEndAction::Fallback { plan_version_id }) => Some(plan_version_id),
            _ => None,
        }
        .filter(|version| {
            current.as_ref().is_none_or(|offer| {
                offer.end_action
                    != TrialEndAction::Fallback {
                        plan_version_id: *version,
                    }
            })
        });
        let without_card = terms
            .without_card
            .and_then(|without_card| without_card.plan_version_id())
            .filter(|version| {
                current.as_ref().is_none_or(|offer| {
                    offer.without_card
                        != TrialWithoutCard::Switch {
                            plan_version_id: *version,
                        }
                })
            });
        for version in at_end.into_iter().chain(without_card) {
            if !crate::services::lifecycle::derived::plan_is_free_tx(&mut conn, version, tenant_id)
                .await?
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

impl ServicesEdge {
    /// The subscription's trial as its customer reads it; none without one, or once it ended
    /// converted, cancelled or switched.
    pub async fn customer_trial(
        &self,
        tenant_id: TenantId,
        details: &crate::domain::SubscriptionDetails,
    ) -> StoreResult<Option<crate::domain::trials::CustomerTrial>> {
        use crate::domain::customer_payment_methods::ResolvedPaymentMethod;
        use crate::domain::trials::{CustomerTrial, TrialEnd, TrialInvoiceToPay, TrialStatus};
        use crate::repositories::customer_payment_methods::CustomerPaymentMethodsInterface;
        let subscription_id = details.subscription.id;
        let trial = {
            let mut conn = self.store.get_conn().await?;
            trial_of_tx(&mut conn, subscription_id).await?
        };
        let Some(trial) = trial else {
            return Ok(None);
        };
        let status = trial.status();
        if !matches!(
            status,
            TrialStatus::Active | TrialStatus::AwaitingPayment | TrialStatus::AwaitingDecision
        ) {
            return Ok(None);
        }
        let after_trial = if status == TrialStatus::Active && trial.end_action == TrialEnd::Convert
        {
            self.after_trial(details).await.unwrap_or_else(|error| {
                log::warn!(
                    "Subscription {subscription_id} is not priced after its trial: {error:?}"
                );
                None
            })
        } else {
            None
        };
        let resolved = self
            .store
            .resolve_payment_method_for_subscription(tenant_id, subscription_id)
            .await?;
        let has_payment_method = !matches!(resolved, ResolvedPaymentMethod::NotConfigured);
        let payment_method = match resolved {
            ResolvedPaymentMethod::CustomerPaymentMethod(id) => Some(
                self.store
                    .get_payment_method_by_id(&tenant_id, &id)
                    .await?
                    .label(),
            ),
            _ => None,
        };
        let invoice_to_pay = if status == TrialStatus::AwaitingPayment {
            let mut conn = self.store.get_conn().await?;
            db(
                diesel_models::invoices::InvoiceRow::list_unsettled_by_subscription(
                    &mut conn,
                    tenant_id,
                    subscription_id,
                    &[],
                )
                .await,
            )?
            .into_iter()
            .find(|invoice| invoice.subscription_id == Some(subscription_id))
            .map(|invoice| TrialInvoiceToPay {
                invoice_id: invoice.id,
                invoice_number: invoice.invoice_number,
                amount_due: invoice.amount_due,
                currency: invoice.currency,
                due_at: invoice.due_at,
            })
        } else {
            None
        };
        Ok(Some(CustomerTrial {
            ends_on: trial.until,
            status,
            after_trial,
            has_payment_method,
            payment_method,
            ended_on: trial.ended_at.map(|at| at.date()),
            invoice_to_pay,
        }))
    }

    /// What `details` bills each period once its trial ends, per cadence; none without a trial.
    pub async fn after_trial(
        &self,
        details: &crate::domain::SubscriptionDetails,
    ) -> StoreResult<Option<crate::domain::trials::AfterTrial>> {
        use crate::domain::invoice_lines::ChargeType;
        let Some(trial) = details.trial_config.as_ref() else {
            return Ok(None);
        };
        let starts_on = match trial.ends_on {
            Some(ends_on) => ends_on,
            None => trial
                .starts_on
                .checked_add_days(chrono::Days::new(u64::from(trial.duration_days)))
                .ok_or_else(|| invalid("the trial ends out of range"))?,
        };
        let mut from = details.clone();
        from.subscription.billing_start_date = Some(starts_on);
        let content = self.compute_new_subscription_invoice(&from).await?;
        let lines = content
            .invoice_lines
            .iter()
            .filter(|line| {
                line.metric_id.is_none()
                    && !matches!(
                        line.charge_type,
                        Some(ChargeType::Usage) | Some(ChargeType::OneTime)
                    )
            })
            .map(|line| (line_period(details, line), line.amount_subtotal));
        Ok(Some(crate::domain::trials::AfterTrial {
            currency: details.subscription.currency.clone(),
            starts_on,
            recurring: per_cadence(details.subscription.period, lines),
        }))
    }

    /// The trial of each of `plans` that has one.
    pub async fn plan_trials(
        &self,
        tenant_id: TenantId,
        plans: &[PlanId],
    ) -> StoreResult<std::collections::HashMap<PlanId, PlanTrial>> {
        let mut conn = self.store.get_conn().await?;
        catalog::plan_trials_tx(&mut conn, tenant_id, plans).await
    }

    /// The plan's trial set to a catalog offer, or removed.
    pub async fn set_plan_trial_offer(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        plan_id: PlanId,
        offer: Option<TrialOfferId>,
    ) -> StoreResult<Option<PlanTrial>> {
        self.services
            .set_plan_trial_offer(actor, tenant_id, plan_id, offer)
            .await
    }

    pub async fn create_trial_offer(
        &self,
        tenant_id: TenantId,
        offer: TrialOfferNew,
    ) -> StoreResult<TrialOffer> {
        self.services.create_trial_offer(tenant_id, offer).await
    }

    /// An offer's terms edited, for trials to come (R-153).
    pub async fn update_trial_offer(
        &self,
        tenant_id: TenantId,
        id: TrialOfferId,
        patch: TrialOfferPatch,
    ) -> StoreResult<TrialOffer> {
        self.services.update_trial_offer(tenant_id, id, patch).await
    }

    /// Archived, an offer is no longer given nor any plan's trial; the plans it was the trial of.
    pub async fn archive_trial_offer(
        &self,
        tenant_id: TenantId,
        id: TrialOfferId,
    ) -> StoreResult<Vec<PlanId>> {
        self.services.archive_trial_offer(tenant_id, id).await
    }

    pub async fn list_trial_offers(&self, tenant_id: TenantId) -> StoreResult<Vec<TrialOffer>> {
        let mut conn = self.store.get_conn().await?;
        db(diesel_models::trial_offers::TrialOfferRow::list(&mut conn, &tenant_id).await)?
            .into_iter()
            .map(TrialOffer::try_from)
            .collect()
    }

    pub async fn list_trial_offers_after(
        &self,
        tenant_id: TenantId,
        after: Option<TrialOfferId>,
        include_archived: bool,
        limit: i64,
    ) -> StoreResult<Vec<TrialOffer>> {
        let mut conn = self.store.get_conn().await?;
        db(diesel_models::trial_offers::TrialOfferRow::list_after(
            &mut conn,
            &tenant_id,
            after,
            include_archived,
            limit,
        )
        .await)?
        .into_iter()
        .map(TrialOffer::try_from)
        .collect()
    }

    /// The tenant's offer of `id`, or coded `code`.
    pub async fn find_trial_offer(
        &self,
        tenant_id: TenantId,
        id_or_code: AliasOr<TrialOfferId>,
    ) -> StoreResult<TrialOffer> {
        let mut conn = self.store.get_conn().await?;
        catalog::find_tx(&mut conn, tenant_id, &id_or_code).await
    }

    /// The plans each of `offers` is the trial of.
    pub async fn plans_of_trial_offers(
        &self,
        tenant_id: TenantId,
        offers: &[TrialOfferId],
    ) -> StoreResult<std::collections::HashMap<TrialOfferId, Vec<PlanId>>> {
        let mut conn = self.store.get_conn().await?;
        catalog::plans_of_tx(&mut conn, tenant_id, offers).await
    }

    /// The subscription's trial, as its fact records it, and the offer it came from.
    pub async fn subscription_trial(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
    ) -> StoreResult<Option<(SubscriptionTrial, Option<TrialOffer>)>> {
        let mut conn = self.store.get_conn().await?;
        let Some(trial) = trial_of_tx(&mut conn, subscription_id).await? else {
            return Ok(None);
        };
        let offer = match trial.offer_id {
            Some(id) => db(diesel_models::trial_offers::TrialOfferRow::find(
                &mut conn, &tenant_id, id,
            )
            .await)?
            .map(TrialOffer::try_from)
            .transpose()?,
            None => None,
        };
        Ok(Some((trial, offer)))
    }

    /// A running trial extended, or ended today (R-153).
    pub async fn move_trial(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        to: TrialMove,
    ) -> StoreResult<SubscriptionTrial> {
        self.services
            .move_trial(actor, tenant_id, subscription_id, to)
            .await
    }
}

/// The cadence a line bills on: its component's or add-on's, else the subscription's.
fn line_period(
    details: &crate::domain::SubscriptionDetails,
    line: &crate::domain::invoice_lines::LineItem,
) -> crate::domain::enums::BillingPeriodEnum {
    let component = line.sub_component_id.and_then(|id| {
        details
            .price_components
            .iter()
            .find(|component| component.id == id)
            .map(|component| component.period)
    });
    let add_on = || {
        line.sub_add_on_id.and_then(|id| {
            details
                .add_ons
                .iter()
                .find(|add_on| add_on.id == id)
                .map(|add_on| add_on.period)
        })
    };
    component
        .or_else(add_on)
        .and_then(|period| period.as_billing_period_opt())
        .unwrap_or(details.subscription.period)
}

/// `lines` summed per cadence, `own` first, then the others shortest first; a cadence summing to
/// nothing is left out.
fn per_cadence(
    own: crate::domain::enums::BillingPeriodEnum,
    lines: impl Iterator<Item = (crate::domain::enums::BillingPeriodEnum, i64)>,
) -> Vec<crate::domain::trials::RecurringAmount> {
    use crate::domain::trials::RecurringAmount;
    let mut recurring: Vec<RecurringAmount> = Vec::new();
    for (period, amount) in lines {
        match recurring.iter_mut().find(|entry| entry.period == period) {
            Some(entry) => entry.amount += amount,
            None => recurring.push(RecurringAmount { amount, period }),
        }
    }
    recurring.retain(|entry| entry.amount != 0);
    recurring.sort_by_key(|entry| (entry.period != own, entry.period.as_months()));
    recurring
}

#[cfg(test)]
mod tests {
    use super::per_cadence;
    use crate::domain::enums::BillingPeriodEnum::{Annual, Monthly, Quarterly};
    use crate::domain::trials::RecurringAmount;

    #[test]
    fn recurring_fees_are_summed_per_cadence_the_subscription_s_first() {
        let lines = [
            (Monthly, 2_000),
            (Annual, 49_000),
            (Quarterly, 0),
            (Monthly, 2_000),
        ];
        assert_eq!(
            per_cadence(Annual, lines.into_iter()),
            vec![
                RecurringAmount {
                    amount: 49_000,
                    period: Annual
                },
                RecurringAmount {
                    amount: 4_000,
                    period: Monthly
                },
            ]
        );
    }

    #[test]
    fn a_plan_of_usage_only_bills_nothing_recurring() {
        assert!(per_cadence(Monthly, [(Monthly, 0)].into_iter()).is_empty());
    }
}
