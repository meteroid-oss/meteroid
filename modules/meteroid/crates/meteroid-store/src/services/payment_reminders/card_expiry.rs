//! Pre-dunning: a customer is told their card expires before an automatic charge fails on it.
use crate::StoreResult;
use crate::domain::customer_emails::{EmailContext, EmailKind};
use crate::errors::StoreError;
use crate::repositories::customer_emails::CustomerEmailsInterface;
use crate::services::{Services, ServicesEdge};
use chrono::NaiveDateTime;
use common_domain::ids::{CustomerId, CustomerPaymentMethodId, TenantId};
use diesel_models::query::customer_payment_methods::{
    ExpiringCardRow, list_expiring_charged_cards, record_expiry_notice,
};
use error_stack::Report;
use scoped_futures::ScopedFutureExt;

/// How long before a card's last valid day its customer is told.
pub const NOTICE_DAYS_BEFORE: i64 = 30;
/// Cards handled per sweep; the rest wait for the next one.
const BATCH: i64 = 500;

impl ServicesEdge {
    /// One pass with an explicit clock; how many customers were told.
    pub async fn run_card_expiry_sweep(&self, now: NaiveDateTime) -> StoreResult<usize> {
        self.services.run_card_expiry_sweep(now).await
    }
}

impl Services {
    pub(crate) async fn run_card_expiry_sweep(&self, now: NaiveDateTime) -> StoreResult<usize> {
        let today = now.date();
        let mut conn = self.store.get_conn().await?;
        let cards = list_expiring_charged_cards(
            &mut conn,
            today,
            today + chrono::Duration::days(NOTICE_DAYS_BEFORE),
            BATCH,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
        drop(conn);

        let mut notified = 0;
        for card in cards {
            let id = card.id;
            match self.notify_expiring_card(card).await {
                Ok(()) => notified += 1,
                Err(e) => log::warn!("Expiring card {id} was not notified: {e:?}"),
            }
        }
        Ok(notified)
    }

    async fn notify_expiring_card(&self, card: ExpiringCardRow) -> StoreResult<()> {
        let tenant_id = TenantId::from(card.tenant_id);
        let payment_method_id = CustomerPaymentMethodId::from(card.id);
        let (exp_year, exp_month) = (card.card_exp_year, card.card_exp_month);
        let context = EmailContext::PaymentMethodExpiring {
            customer_id: CustomerId::from(card.customer_id),
            payment_method_id,
            method: card_label(card.card_brand.as_deref(), card.card_last4.as_deref()),
            exp_month: u32::try_from(exp_month).unwrap_or(1),
            exp_year,
        };
        self.store
            .transaction(|conn| {
                async move {
                    if record_expiry_notice(conn, tenant_id, payment_method_id, exp_year, exp_month)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                    {
                        self.store
                            .enqueue_customer_email_tx(
                                conn,
                                tenant_id,
                                EmailKind::PaymentMethodExpiring,
                                context,
                                format!(
                                    "payment_method_expiring:{payment_method_id}:{exp_year}-{exp_month:02}"
                                ),
                            )
                            .await?;
                    }
                    Ok(())
                }
                .scope_boxed()
            })
            .await
    }
}

/// "Visa •••• 4242", as the customer knows the card.
pub fn card_label(brand: Option<&str>, last4: Option<&str>) -> String {
    let brand = brand
        .filter(|b| !b.trim().is_empty())
        .map(|b| {
            let mut chars = b.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect::<String>())
                .unwrap_or_default()
        })
        .unwrap_or_else(|| "Card".to_owned());
    match last4 {
        Some(last4) => format!("{brand} •••• {last4}"),
        None => brand,
    }
}

#[cfg(test)]
mod tests {
    use super::card_label;

    #[test]
    fn a_card_reads_as_its_brand_and_last_digits() {
        assert_eq!(card_label(Some("visa"), Some("4242")), "Visa •••• 4242");
        assert_eq!(card_label(None, Some("4242")), "Card •••• 4242");
        assert_eq!(card_label(Some("amex"), None), "Amex");
    }
}
