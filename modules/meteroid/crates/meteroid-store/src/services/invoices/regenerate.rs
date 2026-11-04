use crate::StoreResult;
use crate::domain::entity_activity::Actor;
use crate::domain::{
    Customer, DetailedInvoice, Invoice, InvoiceStatusEnum, InvoiceType, SubscriptionStatusEnum,
};
use crate::errors::StoreError;
use crate::repositories::SubscriptionInterface;
use crate::services::invoices::PeriodBilling;
use crate::services::{InvoiceBillingMode, Services, ServicesEdge};
use common_domain::ids::{InvoiceId, TenantId};
use diesel_models::customers::CustomerRow;
use diesel_models::invoices::InvoiceRow;
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::{Report, bail};
use scoped_futures::ScopedFutureExt;

impl Services {
    /// Bills again, on current data, the subscription period a voided recurring invoice billed, or
    /// the last window a voided final invoice billed. Only the period the subscription is in can be
    /// regenerated, as Chargebee's `regenerate_invoice`: a past period's components and minimum are
    /// no longer at hand.
    async fn regenerate_voided_invoice(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<DetailedInvoice> {
        let mut conn = self.store.get_conn().await?;
        let actor = &actor;
        self.transaction_reading_usage_first(&mut conn, |services, conn| {
            async move {
                let subscription_id = InvoiceRow::find_by_id(conn, tenant_id, invoice_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
                    .subscription_id
                    .ok_or_else(|| {
                        Report::new(StoreError::InvalidArgument(
                            "Only a subscription's recurring invoice can be regenerated"
                                .to_string(),
                        ))
                    })?;
                SubscriptionRow::lock_subscription_for_update(conn, subscription_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                let voided: Invoice = InvoiceRow::find_by_id(conn, tenant_id, invoice_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
                    .try_into()?;
                if !voided.invoice_type.bills_subscription_usage() {
                    bail!(StoreError::InvalidArgument(
                        "Only a subscription's recurring invoice can be regenerated".to_string()
                    ));
                }
                if voided.status != InvoiceStatusEnum::Void {
                    bail!(StoreError::InvalidArgument(
                        "Only a voided invoice can be regenerated".to_string()
                    ));
                }

                let details = services
                    .store
                    .get_subscription_details_with_conn(conn, tenant_id, subscription_id)
                    .await?;
                let subscription = &details.subscription;
                let not_billable = match subscription.status {
                    SubscriptionStatusEnum::PendingActivation => Some("has not started yet"),
                    SubscriptionStatusEnum::PendingCharge => Some("awaits its first payment"),
                    SubscriptionStatusEnum::Aborted => Some("was never activated"),
                    SubscriptionStatusEnum::Superseded => {
                        Some("was replaced by another subscription")
                    }
                    SubscriptionStatusEnum::Errored => Some("is in error"),
                    _ if subscription.pending_checkout => Some("awaits its checkout"),
                    _ => None,
                };
                if let Some(state) = not_billable {
                    bail!(StoreError::InvalidArgument(format!(
                        "This invoice cannot be regenerated: its subscription {state}"
                    )));
                }
                let same_cycle = voided
                    .cycle_index
                    .is_none_or(|cycle| subscription.cycle_index == Some(cycle));
                if subscription.current_period_start != voided.billing_period_start || !same_cycle {
                    bail!(StoreError::InvalidArgument(format!(
                        "Only the invoice of the current billing period (starting {}) can be \
                         regenerated; bill a past period with a one-off invoice",
                        subscription.current_period_start.format("%b %-d, %Y")
                    )));
                }

                // The period would be billed again on its new state for all of it, after the
                // change was already charged or credited on its own (Chargebee, Lago).
                if InvoiceRow::subscription_changed_within_period(
                    conn,
                    tenant_id,
                    subscription_id,
                    voided.billing_period_start,
                    voided.created_at,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                {
                    bail!(StoreError::InvalidArgument(
                        "The subscription changed during this billing period, so its invoice \
                         cannot be regenerated; bill the period with a one-off invoice instead"
                            .to_string()
                    ));
                }

                let period_billing = if voided.invoice_type == InvoiceType::Final {
                    if let Some(live) =
                        InvoiceRow::find_final_invoice(conn, tenant_id, subscription_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?
                    {
                        bail!(StoreError::InvalidArgument(format!(
                            "The subscription's last window already has invoice {}",
                            live.id
                        )));
                    }
                    PeriodBilling::Final
                } else {
                    PeriodBilling::RegenerateVoided
                };

                let customer: Customer =
                    CustomerRow::find_by_id(conn, &subscription.customer_id, &tenant_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                        .try_into()?;
                services
                    .bill_subscription_with_data_tx(
                        conn,
                        actor,
                        tenant_id,
                        details,
                        customer,
                        InvoiceBillingMode::AwaitGracePeriodIfApplicable,
                        period_billing,
                    )
                    .await?
                    .ok_or_else(|| {
                        Report::new(StoreError::InvalidArgument(
                            "Nothing is left to bill for this period".to_string(),
                        ))
                    })
            }
            .scope_boxed()
        })
        .await
    }
}

impl ServicesEdge {
    pub async fn regenerate_invoice(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<DetailedInvoice> {
        self.services
            .regenerate_voided_invoice(actor, tenant_id, invoice_id)
            .await
    }

    /// Whether the invoice's period is one `regenerate_invoice` bills again: a voided recurring or
    /// final invoice of the period its subscription is in, which no live invoice bills yet. The
    /// subscription's status and changes are checked when it runs.
    pub async fn can_regenerate_invoice(
        &self,
        tenant_id: TenantId,
        invoice: &Invoice,
    ) -> StoreResult<bool> {
        let Some(subscription_id) = invoice.subscription_id else {
            return Ok(false);
        };
        if invoice.status != InvoiceStatusEnum::Void
            || !invoice.invoice_type.bills_subscription_usage()
        {
            return Ok(false);
        }
        let mut conn = self.services.store.get_conn().await?;
        let subscription =
            SubscriptionRow::get_subscription_by_id(&mut conn, &tenant_id, subscription_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .subscription;
        let same_cycle = invoice
            .cycle_index
            .is_none_or(|cycle| subscription.cycle_index == i32::try_from(cycle).ok());
        if subscription.current_period_start != invoice.billing_period_start || !same_cycle {
            return Ok(false);
        }
        let billed = if invoice.invoice_type == InvoiceType::Final {
            InvoiceRow::find_final_invoice(&mut conn, tenant_id, subscription_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .is_some()
        } else {
            InvoiceRow::find_existing_recurring_invoice(
                &mut conn,
                tenant_id,
                subscription_id,
                invoice.billing_period_start,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .is_some_and(|row| row.status != diesel_models::enums::InvoiceStatusEnum::Void)
        };
        Ok(!billed)
    }
}
