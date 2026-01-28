use crate::StoreResult;
use crate::domain::enums::QuoteStatusEnum;
use crate::domain::quotes::DetailedQuote;
use crate::domain::subscription_add_ons::SubscriptionAddOnNewInternal;
use crate::domain::subscription_components::SubscriptionComponentNewInternal;
use crate::domain::{CreateSubscriptionFromQuote, CreatedSubscription, SubscriptionNew};
use crate::errors::StoreError;
use crate::services::ServicesEdge;
use common_domain::ids::{QuoteId, TenantId};
use error_stack::Report;
use uuid::Uuid;

/// Struct containing the result of a quote to subscription conversion
#[derive(Debug)]
pub struct QuoteConversionResult {
    pub subscription: CreatedSubscription,
}

impl ServicesEdge {
    pub async fn convert_quote_to_subscription(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
        created_by: Uuid,
    ) -> StoreResult<QuoteConversionResult> {
        use crate::repositories::QuotesInterface;

        // Fetch the detailed quote
        let detailed_quote = self
            .store
            .get_detailed_quote_by_id(tenant_id, quote_id)
            .await?;

        // Validate the quote is accepted
        if detailed_quote.quote.status != QuoteStatusEnum::Accepted {
            return Err(Report::new(StoreError::InvalidArgument(format!(
                "Quote must be in Accepted status to convert to subscription. Current status: {:?}",
                detailed_quote.quote.status
            ))));
        }

        // Check if quote is already converted
        if detailed_quote.quote.converted_to_subscription_id.is_some() {
            return Err(Report::new(StoreError::InvalidArgument(
                "Quote has already been converted to a subscription".to_string(),
            )));
        }

        let create_subscription = build_subscription_from_quote(&detailed_quote, created_by)?;

        // Create the subscription using the quote-specific method
        // This bypasses plan-based component/add-on processing since they're already processed
        let created = self
            .insert_subscription_from_quote(create_subscription, tenant_id)
            .await?;

        Ok(QuoteConversionResult {
            subscription: created,
        })
    }
}

fn build_subscription_from_quote(
    detailed_quote: &DetailedQuote,
    created_by: Uuid,
) -> StoreResult<CreateSubscriptionFromQuote> {
    let quote = &detailed_quote.quote;
    let now = chrono::Utc::now().naive_utc().date();

    // If billing_start_date is not set, use today's date (dynamic start date)
    let start_date = quote.billing_start_date.unwrap_or(now);

    let subscription_new = SubscriptionNew {
        customer_id: quote.customer_id,
        plan_version_id: quote.plan_version_id,
        created_by,
        net_terms: Some(quote.net_terms as u32),
        invoice_memo: quote.invoice_memo.clone(),
        invoice_threshold: quote.invoice_threshold,
        start_date,
        end_date: quote.billing_end_date,
        billing_start_date: Some(start_date),
        activation_condition: quote.activation_condition.clone(),
        trial_duration: quote.trial_duration_days.map(|d| d as u32),
        billing_day_anchor: quote.billing_day_anchor.map(|d| d as u16),
        payment_strategy: Some(quote.payment_strategy.clone()),
        auto_advance_invoices: quote.auto_advance_invoices,
        charge_automatically: quote.charge_automatically,
        purchase_order: quote.purchase_order.clone(),
        backdate_invoices: false,
        skip_checkout_session: false,
    };

    let components: Vec<SubscriptionComponentNewInternal> = detailed_quote
        .components
        .iter()
        .map(|c| SubscriptionComponentNewInternal {
            price_component_id: c.price_component_id,
            product_id: c.product_id,
            name: c.name.clone(),
            period: c.period,
            fee: c.fee.clone(),
            is_override: c.is_override,
        })
        .collect();

    let add_ons: Vec<SubscriptionAddOnNewInternal> = detailed_quote
        .add_ons
        .iter()
        .map(|a| SubscriptionAddOnNewInternal {
            add_on_id: a.add_on_id,
            name: a.name.clone(),
            period: a.period,
            fee: a.fee.clone(),
        })
        .collect();

    let coupon_ids = detailed_quote.coupons.iter().map(|c| c.coupon_id).collect();

    Ok(CreateSubscriptionFromQuote {
        subscription: subscription_new,
        components,
        add_ons,
        coupon_ids,
        quote_id: quote.id,
    })
}
