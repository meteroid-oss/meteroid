//! The next invoice of a subscription as the portal shows it, from one computation of it.
use crate::invoices::mapping::invoices::{
    domain_coupon_line_item_to_server, domain_invoice_lines_to_server,
};
use common_domain::ids::{SubscriptionId, TenantId};
use meteroid_app::conversions::ProtoConv;
use meteroid_grpc::meteroid::portal::subscription::v1::{PortalBilledFirst, PortalUpcomingInvoice};
use meteroid_store::Store;
use meteroid_store::domain::credit_outlook::BilledFirst;
use meteroid_store::domain::customer_payment_methods::ResolvedPaymentMethod;
use meteroid_store::domain::portal_credit::recurring_issue_and_due;
use meteroid_store::domain::{CustomerPaymentMethod, SubscriptionDetails};
use meteroid_store::repositories::customer_payment_methods::CustomerPaymentMethodsInterface;
use meteroid_store::services::invoice_lines::invoice_lines::ComputedInvoiceContent;
use tonic::Status;

/// The card or account the next invoice is charged to. Shown for information only, so an
/// unreadable one is left out rather than failing the upcoming invoice.
async fn charged_payment_method(
    store: &Store,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
) -> Option<CustomerPaymentMethod> {
    let read = async {
        match store
            .resolve_payment_method_for_subscription(tenant_id, subscription_id)
            .await?
        {
            ResolvedPaymentMethod::CustomerPaymentMethod(id) => store
                .get_payment_method_by_id(&tenant_id, &id)
                .await
                .map(Some),
            ResolvedPaymentMethod::BankTransfer(_) | ResolvedPaymentMethod::NotConfigured => {
                Ok(None)
            }
        }
    };
    read.await
        .inspect_err(|error| {
            log::warn!("payment method of subscription {subscription_id} unavailable: {error:?}");
        })
        .ok()
        .flatten()
}

/// `invoice_date` is the forecast's: the period end, or the day the subscription ends.
pub async fn upcoming_invoice_view(
    store: &Store,
    details: &SubscriptionDetails,
    invoice_date: chrono::NaiveDate,
    content: ComputedInvoiceContent,
    credit_estimate_shared: bool,
    billed_first: &[BilledFirst],
) -> Result<PortalUpcomingInvoice, Status> {
    let subscription = &details.subscription;
    let entity = &details.invoicing_entity;
    let (charge_date, due_date) = recurring_issue_and_due(
        invoice_date,
        subscription.net_terms,
        entity.grace_period_hours,
        entity.consolidate_recurring_invoices,
    );
    // Left as draft, an invoice is never charged on its own.
    let payment_method = if subscription.charge_automatically && subscription.auto_advance_invoices
    {
        charged_payment_method(store, subscription.tenant_id, subscription.id).await
    } else {
        None
    };
    Ok(PortalUpcomingInvoice {
        subscription_id: subscription.id.as_proto(),
        charge_date: charge_date.as_proto(),
        due_date: due_date.as_proto(),
        charge_automatically: payment_method.is_some(),
        payment_method: payment_method
            .map(crate::customers::mapping::customer_payment_method::domain_to_server),
        currency: subscription.currency.clone(),
        invoice_date: invoice_date.as_proto(),
        line_items: domain_invoice_lines_to_server(content.invoice_lines),
        coupon_line_items: domain_coupon_line_item_to_server(content.applied_coupons),
        subtotal: content.subtotal,
        tax_amount: content.tax_amount,
        total: content.total,
        amount_due: content.amount_due,
        discount: content.discount,
        promotional_credits: content.promotional_credits,
        period_start: subscription.current_period_start.as_proto(),
        period_end: subscription
            .current_period_end
            .map(|d| d.as_proto())
            .unwrap_or_default(),
        plan_name: Some(subscription.plan_name.clone()),
        credit_estimate_shared,
        billed_first: billed_first
            .iter()
            .map(|first| PortalBilledFirst {
                subscription_id: first.subscription_id.map(|id| id.as_proto()),
                plan_name: first.plan_name.clone(),
                invoice_date: first.invoice_date.as_proto(),
            })
            .collect(),
    })
}
