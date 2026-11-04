//! Which subscription an invoice line bills, and so whose credit may pay it: the line's own record,
//! else the subscription of its component or add-on (historical rows persist across plan changes),
//! else the invoice's. Consolidated parents and lines stored before the record rely on the lookups.
use crate::StoreResult;
use crate::domain::LineItem;
use crate::errors::StoreError;
use crate::services::credit::invalid;
use crate::store::PgConn;
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use common_domain::ids::{
    BaseId, CustomerId, SubscriptionAddOnId, SubscriptionId, SubscriptionPriceComponentId, TenantId,
};
use diesel_models::enums::SubscriptionStatusEnum;
use diesel_models::subscription_add_ons::SubscriptionAddOnRow;
use diesel_models::subscription_components::SubscriptionComponentRow;
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::Report;
use std::collections::HashMap;

/// The subscriptions of the components and add-ons a document's lines name, loaded once.
#[derive(Debug, Clone, Default)]
pub struct LineOwners {
    by_component: HashMap<SubscriptionPriceComponentId, SubscriptionId>,
    by_add_on: HashMap<SubscriptionAddOnId, SubscriptionId>,
}

impl LineOwners {
    /// Reads only what lines without their own record need; most documents need nothing.
    pub async fn load_tx(
        conn: &mut PgConn,
        tenant_id: TenantId,
        lines: &[LineItem],
    ) -> StoreResult<Self> {
        let unrecorded = || lines.iter().filter(|line| line.subscription_id.is_none());
        let mut components = unrecorded()
            .filter_map(|line| line.sub_component_id)
            .collect::<Vec<_>>();
        components.sort_by_key(|id| id.as_uuid());
        components.dedup();
        let mut add_ons = unrecorded()
            .filter(|line| line.sub_component_id.is_none())
            .filter_map(|line| line.sub_add_on_id)
            .collect::<Vec<_>>();
        add_ons.sort_by_key(|id| id.as_uuid());
        add_ons.dedup();
        Ok(Self {
            by_component: SubscriptionComponentRow::subscription_ids_by_ids(
                conn,
                tenant_id,
                &components,
            )
            .await?,
            by_add_on: SubscriptionAddOnRow::subscription_ids_by_ids(conn, tenant_id, &add_ons)
                .await?,
        })
    }
}

#[cfg(test)]
impl LineOwners {
    pub(crate) fn for_components(
        owners: impl IntoIterator<Item = (SubscriptionPriceComponentId, SubscriptionId)>,
    ) -> Self {
        Self {
            by_component: owners.into_iter().collect(),
            by_add_on: HashMap::new(),
        }
    }
}

/// The subscription `line` bills; None for a line of no subscription (a credit purchase, a sale).
pub fn line_subscription(
    line: &LineItem,
    invoice_subscription: Option<SubscriptionId>,
    owners: &LineOwners,
) -> Option<SubscriptionId> {
    line.subscription_id
        .or_else(|| {
            line.sub_component_id
                .and_then(|id| owners.by_component.get(&id).copied())
        })
        .or_else(|| {
            line.sub_add_on_id
                .and_then(|id| owners.by_add_on.get(&id).copied())
        })
        .or(invoice_subscription)
}

/// Statuses that end a subscription's plan credit; a pause, suspension, expired trial or error
/// leaves it to its own expiry.
pub(crate) fn ends_credit(status: &SubscriptionStatusEnum) -> bool {
    matches!(
        status,
        SubscriptionStatusEnum::Cancelled
            | SubscriptionStatusEnum::Completed
            | SubscriptionStatusEnum::Aborted
            | SubscriptionStatusEnum::Superseded
    )
}

/// When a subscription's credit ends, None while it has not ended. A terminal transition opens
/// its last period at the date it took effect; an immediate cancellation also records its exact
/// instant, where the final usage window ends.
pub(crate) fn credit_ended_at(subscription: &SubscriptionRow) -> Option<DateTime<Utc>> {
    ended_at(
        &subscription.status,
        subscription.terminated_at,
        subscription.current_period_start,
    )
}

fn ended_at(
    status: &SubscriptionStatusEnum,
    terminated_at: Option<NaiveDateTime>,
    current_period_start: NaiveDate,
) -> Option<DateTime<Utc>> {
    ends_credit(status).then(|| {
        terminated_at.map_or_else(
            || current_period_start.and_time(NaiveTime::MIN).and_utc(),
            |at| at.and_utc(),
        )
    })
}

/// A subscription a grant may belong to: the customer's own.
#[derive(Debug, Clone)]
pub(crate) struct OwningSubscription {
    pub currency: String,
    /// In a status that ends its credit.
    pub ended: bool,
    /// When it ended: credit funded for it after that ends with it.
    pub ended_at: Option<DateTime<Utc>>,
}

pub(crate) async fn owning_subscription_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
    subscription_id: SubscriptionId,
) -> StoreResult<OwningSubscription> {
    let row = SubscriptionRow::get_subscription_by_id(conn, &tenant_id, subscription_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    if row.subscription.customer_id != customer_id {
        return Err(invalid("the subscription belongs to another customer"));
    }
    Ok(OwningSubscription {
        ended: ends_credit(&row.subscription.status),
        ended_at: credit_ended_at(&row.subscription),
        currency: row.subscription.currency,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use common_domain::ids::BaseId;
    use rust_decimal::Decimal;

    fn line() -> LineItem {
        let day = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        LineItem {
            local_id: "l".to_owned(),
            name: "line".to_owned(),
            charge_type: None,
            amount_subtotal: 100,
            tax_rate: Decimal::ZERO,
            taxable_amount: 100,
            tax_amount: 0,
            amount_total: 100,
            tax_details: Vec::new(),
            quantity: None,
            unit_price: None,
            start_date: day,
            end_date: day,
            sub_lines: Vec::new(),
            is_prorated: false,
            price_component_id: None,
            sub_component_id: None,
            sub_add_on_id: None,
            product_id: None,
            metric_id: None,
            description: None,
            group_by_dimensions: None,
            kind: crate::domain::LineItemKind::Regular,
            credit_grant_amount: None,
            credit_grant_expiry: None,
            credit_grant_included: false,
            taxed_at_purchase: false,
            tax_inclusive_quote: None,
            credit_unit: None,
            credit_grant_unit: None,
            credit_grant_rollover: None,
            promotional_credit: None,
            subscription_id: None,
            usage_ends_at: None,
        }
    }

    /// T14: a line stored without its subscription, as an older release writes it, resolves
    /// through its component or add-on before the invoice's own.
    #[test]
    fn old_invoice_lines_resolve_their_subscription() {
        let (recorded, by_component, by_add_on, invoice) = (
            SubscriptionId::new(),
            SubscriptionId::new(),
            SubscriptionId::new(),
            SubscriptionId::new(),
        );
        let (component, add_on) = (
            SubscriptionPriceComponentId::new(),
            SubscriptionAddOnId::new(),
        );
        let owners = LineOwners {
            by_component: HashMap::from([(component, by_component)]),
            by_add_on: HashMap::from([(add_on, by_add_on)]),
        };
        let stored = serde_json::to_value(LineItem {
            sub_component_id: Some(component),
            ..line()
        })
        .unwrap();
        assert!(
            stored.get("subscription_id").is_none(),
            "not written when absent"
        );
        let old: LineItem = serde_json::from_value(stored).unwrap();
        assert_eq!(
            line_subscription(&old, Some(invoice), &owners),
            Some(by_component)
        );
        let on_add_on = LineItem {
            sub_add_on_id: Some(add_on),
            ..line()
        };
        assert_eq!(
            line_subscription(&on_add_on, None, &owners),
            Some(by_add_on)
        );
        assert_eq!(
            line_subscription(&line(), Some(invoice), &owners),
            Some(invoice)
        );
        assert_eq!(line_subscription(&line(), None, &owners), None);
        let current = LineItem {
            subscription_id: Some(recorded),
            sub_component_id: Some(component),
            ..line()
        };
        assert_eq!(
            line_subscription(&current, Some(invoice), &owners),
            Some(recorded)
        );
    }

    /// A date-effective cancellation records no instant: its credit ends at the date it took
    /// effect, where its last period opens, not at a later contractual end.
    #[test]
    fn credit_ends_when_the_termination_took_effect() {
        let day = |d| NaiveDate::from_ymd_opt(2026, 3, d).unwrap();
        let midnight = |d| day(d).and_time(NaiveTime::MIN).and_utc();
        assert_eq!(
            ended_at(&SubscriptionStatusEnum::Cancelled, None, day(20)),
            Some(midnight(20))
        );
        let instant = day(20).and_hms_opt(14, 30, 0).unwrap();
        assert_eq!(
            ended_at(&SubscriptionStatusEnum::Cancelled, Some(instant), day(20)),
            Some(instant.and_utc())
        );
        assert_eq!(
            ended_at(&SubscriptionStatusEnum::Completed, None, day(31)),
            Some(midnight(31))
        );
        assert_eq!(
            ended_at(&SubscriptionStatusEnum::Paused, None, day(20)),
            None
        );
        assert_eq!(
            ended_at(&SubscriptionStatusEnum::Active, None, day(20)),
            None
        );
    }

    #[test]
    fn only_terminal_statuses_end_plan_credit() {
        for status in [
            SubscriptionStatusEnum::Cancelled,
            SubscriptionStatusEnum::Completed,
            SubscriptionStatusEnum::Aborted,
            SubscriptionStatusEnum::Superseded,
        ] {
            assert!(ends_credit(&status), "{status:?}");
        }
        for status in [
            SubscriptionStatusEnum::Paused,
            SubscriptionStatusEnum::Suspended,
            SubscriptionStatusEnum::TrialExpired,
            SubscriptionStatusEnum::Errored,
            SubscriptionStatusEnum::Active,
            SubscriptionStatusEnum::TrialActive,
            SubscriptionStatusEnum::PendingActivation,
            SubscriptionStatusEnum::PendingCharge,
        ] {
            assert!(!ends_credit(&status), "{status:?}");
        }
    }
}
