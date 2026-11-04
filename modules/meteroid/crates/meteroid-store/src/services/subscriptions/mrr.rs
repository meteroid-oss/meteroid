use crate::StoreResult;
use crate::domain::SubscriptionDetails;
use crate::errors::StoreError;
use crate::services::Services;
use crate::services::credit::coupon_grant::has_started;
use crate::services::subscriptions::insert::context::QuotedLine;
use crate::services::subscriptions::utils::{MrrLine, subscription_mrr, with_add_on_lines};
use crate::store::PgConn;
use chrono::NaiveDate;
use common_domain::ids::{SubscriptionId, TenantId};
use diesel_models::bi::BiMrrMovementLogRowNew;
use diesel_models::enums::{MrrMovementType, SubscriptionEventType};
use diesel_models::subscription_events::SubscriptionEventRow;
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::Report;
use rust_decimal::Decimal;
use uuid::Uuid;

/// A movement of the subscription's MRR that no invoice books: a change, a churn, or a creation
/// whose first period issued no invoice.
pub(in crate::services) fn subscription_movement(
    subscription: &SubscriptionRow,
    movement_type: MrrMovementType,
    description: &str,
    net_mrr_change: i64,
    applies_to: NaiveDate,
) -> BiMrrMovementLogRowNew {
    BiMrrMovementLogRowNew {
        id: Uuid::now_v7(),
        description: description.to_string(),
        movement_type,
        net_mrr_change,
        currency: subscription.currency.clone(),
        applies_to,
        invoice_id: None,
        credit_note_id: None,
        plan_version_id: subscription.plan_version_id,
        tenant_id: subscription.tenant_id,
        subscription_id: subscription.id,
        customer_id: subscription.customer_id,
    }
}

impl Services {
    /// Books the `Created` event as New Business once the subscription started and billed its
    /// first period, when that period issued no invoice for `process_mrr` to book it on (fees
    /// all usage-based or billed in arrears).
    pub(in crate::services) async fn book_created_mrr_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
    ) -> StoreResult<()> {
        let subscription =
            SubscriptionRow::get_subscription_by_id(conn, &tenant_id, subscription_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .subscription;
        if !has_started(&subscription) {
            return Ok(());
        }
        let Some(event) = SubscriptionEventRow::fetch_by_subscription_id_and_event_type(
            conn,
            subscription_id,
            SubscriptionEventType::Created,
            subscription.start_date,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        else {
            return Ok(());
        };
        let delta = match event.mrr_delta {
            Some(delta) if delta != 0 && event.bi_mrr_movement_log_id.is_none() => delta,
            _ => return Ok(()),
        };
        let log = subscription_movement(
            &subscription,
            MrrMovementType::NewBusiness,
            "Subscription created",
            delta,
            event.applies_to,
        )
        .insert(conn)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
        SubscriptionEventRow::update_mrr_movement_log_id(conn, event.id, log.id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        SubscriptionRow::update_subscription_mrr_delta(conn, subscription_id, delta)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        Ok(())
    }

    /// The MRR a subscription stores once its fees changed. A tax-inclusive subscription's rate
    /// is re-resolved on the lines it now has, and persisted when it moved, so a taxed fee added
    /// where none was taxed, or a mix that changed, nets like one it was created with.
    pub(in crate::services) async fn restate_subscription_mrr(
        &self,
        conn: &mut PgConn,
        sub_details: &mut SubscriptionDetails,
        component_lines: Vec<MrrLine>,
        precision: u8,
        date: NaiveDate,
    ) -> StoreResult<i64> {
        let lines = with_add_on_lines(sub_details, component_lines, precision);
        let rate = self
            .tax_inclusive_rate_for(conn, sub_details, &lines, date)
            .await?;
        if rate != sub_details.subscription.tax_inclusive_rate {
            SubscriptionRow::update_tax_inclusive_rate(
                conn,
                sub_details.subscription.tenant_id,
                sub_details.subscription.id,
                rate,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
            sub_details.subscription.tax_inclusive_rate = rate;
        }
        Ok(subscription_mrr(
            &lines,
            sub_details
                .minimum
                .as_ref()
                .map(|min| (min.amount_subunit, &min.scope)),
            sub_details.subscription.period,
            rate,
        ))
    }

    /// The rate the subscription's prices carry once its MRR lines are `lines`: re-resolved
    /// for a tax-inclusive subscription, with the minimum's shortfall taxed like its invoice
    /// lines (`minimum::split_shortfall`); none for a tax-exclusive one.
    pub(in crate::services) async fn tax_inclusive_rate_for(
        &self,
        conn: &mut PgConn,
        sub_details: &SubscriptionDetails,
        lines: &[MrrLine],
        date: NaiveDate,
    ) -> StoreResult<Option<Decimal>> {
        if !sub_details.subscription.tax_inclusive {
            return Ok(sub_details.subscription.tax_inclusive_rate);
        }
        let period = sub_details.subscription.period;
        let minimum = sub_details
            .minimum
            .as_ref()
            .map(|min| (min.amount_subunit, &min.scope));
        let mut quoted: Vec<QuotedLine> = lines
            .iter()
            .map(|line| QuotedLine {
                product_id: line.product_id,
                period_total_cents: line.period_total,
                monthly_share_cents: line.monthly,
                taxed: line.taxed(),
            })
            .collect();
        let shortfall = subscription_mrr(lines, minimum, period, None)
            - subscription_mrr(lines, None, period, None);
        if shortfall > 0
            && let Some((_, scope)) = minimum
        {
            let weights: Vec<_> = lines
                .iter()
                .filter_map(|line| line.product_id.map(|product| (product, line.monthly)))
                .collect();
            let carried: Vec<_> = lines.iter().filter_map(|line| line.product_id).collect();
            for (product_id, share) in crate::services::invoice_lines::minimum::split_shortfall(
                scope, &weights, &carried, shortfall,
            ) {
                quoted.push(QuotedLine {
                    product_id,
                    period_total_cents: share * i64::from(period.as_months()).max(1),
                    monthly_share_cents: share,
                    taxed: true,
                });
            }
        }
        self.resolve_tax_inclusive_rate(
            conn,
            &sub_details.invoicing_entity,
            &sub_details.customer,
            &sub_details.subscription.currency,
            &quoted,
            date,
        )
        .await
    }
}
