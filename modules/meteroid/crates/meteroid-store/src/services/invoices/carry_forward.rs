//! Carry forward: an amount due too small for the payment providers to collect is not charged but
//! owed on the customer's next invoice, after tax (Stripe's minimum-charge behaviour); a negative
//! total is never refunded but credited to the customer's balance (see finalization).
use crate::StoreResult;
use crate::domain::Invoice;
use crate::domain::enums::PaymentProvider;
use crate::domain::fx::FxTable;
use crate::errors::StoreError;
use crate::repositories::historical_rates::get_historical_rate_from_usd_by_date_cached;
use crate::services::credit::value_application;
use chrono::{DateTime, Utc};
use common_domain::ids::{ConnectorId, TenantId};
use diesel_models::PgConn;
use diesel_models::connectors::ConnectorRow;
use diesel_models::invoice_carry_forward::{
    CarryTarget, InvoiceCarryForwardRow, InvoiceCarryForwardRowNew,
};
use diesel_models::invoices::InvoiceRow;
use diesel_models::invoicing_entities::InvoicingEntityRow;
use diesel_models::lock::CustomerLock;
use error_stack::Report;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

/// The smallest charge each provider accepts, in major units. Stripe:
/// <https://docs.stripe.com/currencies#minimum-and-maximum-charge-amounts>.
fn provider_minimum_major(provider: PaymentProvider, currency: &str) -> Option<Decimal> {
    match provider {
        PaymentProvider::Stripe => Some(match currency {
            "USD" | "AUD" | "BRL" | "CAD" | "CHF" | "EUR" | "INR" | "NZD" | "SGD" => dec!(0.50),
            "GBP" => dec!(0.30),
            "AED" | "MYR" | "PLN" | "RON" => dec!(2.00),
            "BGN" => dec!(1.00),
            "CZK" => dec!(15.00),
            "DKK" => dec!(2.50),
            "HKD" => dec!(4.00),
            "HUF" => dec!(175.00),
            "JPY" => dec!(50),
            "MXN" | "THB" => dec!(10.00),
            "NOK" | "SEK" => dec!(3.00),
            _ => return None,
        }),
        PaymentProvider::Gocardless => Some(dec!(1.00)),
        PaymentProvider::Stancer => (currency == "EUR").then_some(dec!(0.50)),
        PaymentProvider::Mollie | PaymentProvider::Mock => None,
    }
}

/// The smallest charge `provider` accepts in `currency`, in minor units.
pub fn provider_minimum_charge(provider: PaymentProvider, currency: &str) -> Option<i64> {
    let currency = currency.to_ascii_uppercase();
    let precision = crate::constants::Currencies::resolve_currency_precision(&currency)?;
    let major = provider_minimum_major(provider, &currency)?;
    (major * Decimal::from(10_i64.pow(u32::from(precision))))
        .trunc()
        .try_into()
        .ok()
}

/// Below this amount due, an invoice of `entity` in `currency` carries forward. See
/// [`min_charge_amount`]; the entity's floor converts at the rates in force on `on`.
pub(crate) async fn min_charge_amount_tx(
    conn: &mut PgConn,
    entity: &InvoicingEntityRow,
    currency: &str,
    on: chrono::NaiveDate,
) -> StoreResult<i64> {
    let mut providers = Vec::new();
    for connector_id in [entity.card_provider_id, entity.direct_debit_provider_id]
        .into_iter()
        .flatten()
    {
        providers.extend(connector_provider(conn, connector_id, entity.tenant_id).await?);
    }
    let fx = if entity.accounting_currency.eq_ignore_ascii_case(currency) {
        None
    } else {
        get_historical_rate_from_usd_by_date_cached(conn, on)
            .await?
            .map(|rates| FxTable::from_usd_rates(&currency.to_ascii_uppercase(), &rates.rates))
    };
    Ok(min_charge_amount(
        entity.min_charge_amount,
        &providers,
        &entity.accounting_currency,
        currency,
        fx.as_ref(),
    ))
}

/// The highest of the providers' minimums in the invoice currency and, converted from the
/// entity's currency, of its own floor and the providers' minimums there: Stripe holds a charge
/// in another currency to its settlement currency's minimum. `fx` converts into the invoice
/// currency; without it, only amounts already in that currency count.
pub(crate) fn min_charge_amount(
    entity_floor: Option<i64>,
    providers: &[PaymentProvider],
    entity_currency: &str,
    currency: &str,
    fx: Option<&FxTable>,
) -> i64 {
    let providers_in = |currency: &str| {
        providers
            .iter()
            .filter_map(|provider| provider_minimum_charge(*provider, currency))
            .max()
            .unwrap_or(0)
    };
    let entity_minimum = entity_floor.unwrap_or(0).max(providers_in(entity_currency));
    let converted = if entity_currency.eq_ignore_ascii_case(currency) {
        Some(entity_minimum)
    } else {
        fx.and_then(|fx| fx.convert_up(entity_minimum, &entity_currency.to_ascii_uppercase()))
    };
    converted.unwrap_or(0).max(providers_in(currency))
}

/// Whether an amount due is below what the entity's payment providers can charge: such an
/// invoice is left open, neither charged automatically nor chased, for the operator to settle.
pub(crate) async fn below_provider_minimum_tx(
    conn: &mut PgConn,
    invoice: &InvoiceRow,
) -> StoreResult<bool> {
    if invoice.amount_due <= 0 {
        return Ok(false);
    }
    let entity = InvoicingEntityRow::get_invoicing_entity_by_id_and_tenant(
        conn,
        invoice.invoicing_entity_id,
        invoice.tenant_id,
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)?;
    let providers_only = InvoicingEntityRow {
        min_charge_amount: None,
        ..entity
    };
    let minimum = min_charge_amount_tx(
        conn,
        &providers_only,
        &invoice.currency,
        invoice.invoice_date,
    )
    .await?;
    Ok(invoice.amount_due < minimum)
}

async fn connector_provider(
    conn: &mut PgConn,
    id: ConnectorId,
    tenant_id: TenantId,
) -> StoreResult<Option<PaymentProvider>> {
    let connector = ConnectorRow::get_connector_by_id(conn, id, tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    let provider: crate::domain::enums::ConnectorProviderEnum = connector.provider.into();
    Ok(provider.as_payment())
}

/// What a finalizing invoice carried in and out.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct CarriedBalances {
    pub previous_balance: i64,
    pub carried_forward: i64,
    pub amount_due: i64,
}

impl CarriedBalances {
    /// The decision once the debts carried in are known: what a negative total leaves after
    /// settling them is carried as credit; a positive amount due under `minimum` as debt, when a
    /// later invoice will take it.
    pub(crate) fn decide(
        total: i64,
        amount_due: i64,
        previous_balance: i64,
        minimum: i64,
        later_invoice: bool,
    ) -> Self {
        let owed = if total < 0 {
            total.saturating_add(previous_balance)
        } else {
            amount_due.saturating_add(previous_balance)
        };
        let carried_forward = if owed < 0 || (owed > 0 && owed < minimum && later_invoice) {
            owed
        } else {
            0
        };
        Self {
            previous_balance,
            carried_forward,
            amount_due: if carried_forward == 0 { owed.max(0) } else { 0 },
        }
    }

    /// What the invoice stores: the debts it took less the debt it carried out.
    pub(crate) fn carried_balance(&self) -> i64 {
        self.previous_balance - self.carried_forward.max(0)
    }
}

/// The days a finalizing invoice decides on: the rates in force when it is issued, and the end of
/// the latest period its refreshed lines bill.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CarryDates {
    pub issued_on: chrono::NaiveDate,
    pub billed_through: chrono::NaiveDate,
}

/// Takes the customer's carried debts onto a subscription invoice being finalized, then carries
/// its own amount due forward when it is below the minimum charge. Holds the customer lock.
pub(crate) async fn carry_balances_tx(
    conn: &mut PgConn,
    lock: &CustomerLock,
    invoice: &Invoice,
    total: i64,
    amount_due: i64,
    dates: CarryDates,
    at: DateTime<Utc>,
) -> StoreResult<CarriedBalances> {
    let collects_cash =
        value_application::committed_cash_minor(conn, invoice.tenant_id, invoice.id).await? == 0;
    if !invoice.invoice_type.bills_subscription_usage() || !collects_cash {
        let balances = CarriedBalances::decide(total, amount_due, 0, 0, false);
        if balances.carried_forward != 0 {
            record(conn, invoice, balances).await?;
        }
        return Ok(balances);
    }

    let target = CarryTarget {
        invoicing_entity_id: invoice.invoicing_entity_id,
        currency: &invoice.currency,
    };
    let previous_balance =
        InvoiceCarryForwardRow::apply_pending(conn, lock, target, invoice.id, at)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

    let entity = InvoicingEntityRow::get_invoicing_entity_by_id_and_tenant(
        conn,
        invoice.invoicing_entity_id,
        invoice.tenant_id,
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)?;
    let minimum = min_charge_amount_tx(conn, &entity, &invoice.currency, dates.issued_on).await?;
    let candidate = if total < 0 { total } else { amount_due }.saturating_add(previous_balance);
    let later_invoice = candidate > 0
        && candidate < minimum
        && InvoiceCarryForwardRow::customer_has_later_invoice(
            conn,
            invoice.tenant_id,
            invoice.customer_id,
            &invoice.currency,
            dates.billed_through,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

    let balances =
        CarriedBalances::decide(total, amount_due, previous_balance, minimum, later_invoice);
    if balances.carried_forward > 0 {
        InvoiceCarryForwardRowNew {
            id: uuid::Uuid::now_v7(),
            tenant_id: invoice.tenant_id,
            customer_id: invoice.customer_id,
            invoicing_entity_id: invoice.invoicing_entity_id,
            currency: invoice.currency.clone(),
            amount: balances.carried_forward,
            source_invoice_id: invoice.id,
        }
        .insert(conn, lock)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    }
    if balances.carried_forward != 0 || balances.previous_balance != 0 {
        record(conn, invoice, balances).await?;
    }
    Ok(balances)
}

/// Whether closing this empty invoice would strand carried debts no later invoice will take: the
/// customer's last invoice collects them instead.
pub(crate) async fn takes_stranded_debts_tx(
    conn: &mut PgConn,
    invoice: &Invoice,
) -> StoreResult<bool> {
    let target = CarryTarget {
        invoicing_entity_id: invoice.invoicing_entity_id,
        currency: &invoice.currency,
    };
    let pending =
        InvoiceCarryForwardRow::sum_pending(conn, invoice.tenant_id, invoice.customer_id, target)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
    if pending <= 0 {
        return Ok(false);
    }
    let later = InvoiceCarryForwardRow::customer_has_later_invoice(
        conn,
        invoice.tenant_id,
        invoice.customer_id,
        &invoice.currency,
        billed_through(
            invoice.line_items.iter().map(|line| line.end_date),
            invoice.invoice_date,
        ),
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)?;
    Ok(!later)
}

/// Once a subscription ended, the debts no invoice will take any more go back to the invoices
/// that carried them, open again to be collected or written off. Returns those invoices.
pub(crate) async fn reopen_stranded_debts_tx(
    conn: &mut PgConn,
    lock: &CustomerLock,
    currency: &str,
    ended_on: chrono::NaiveDate,
) -> StoreResult<Vec<common_domain::ids::InvoiceId>> {
    let (tenant_id, customer_id) = (lock.tenant_id(), lock.customer_id());
    let taken =
        InvoiceCarryForwardRow::customer_has_draft_in(conn, tenant_id, customer_id, currency)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            || InvoiceCarryForwardRow::customer_has_later_invoice(
                conn,
                tenant_id,
                customer_id,
                currency,
                ended_on,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
    if taken {
        return Ok(Vec::new());
    }
    let debts = InvoiceCarryForwardRow::take_back_pending(conn, lock, currency)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    for (source, amount) in &debts {
        InvoiceRow::adjust_carried_balance(conn, *source, tenant_id, *amount)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        crate::repositories::invoice_account::settle_tx(conn, tenant_id, *source, None).await?;
    }
    Ok(debts.into_iter().map(|(source, _)| source).collect())
}

/// The end of the latest period lines ending on `ends` bill, never before `invoice_date`: a later
/// invoice exists only for a subscription still running past it.
pub(crate) fn billed_through(
    ends: impl Iterator<Item = chrono::NaiveDate>,
    invoice_date: chrono::NaiveDate,
) -> chrono::NaiveDate {
    ends.max().map_or(invoice_date, |end| end.max(invoice_date))
}

async fn record(
    conn: &mut PgConn,
    invoice: &Invoice,
    balances: CarriedBalances,
) -> StoreResult<()> {
    InvoiceRow::set_carried_balance(
        conn,
        invoice.id,
        invoice.tenant_id,
        balances.carried_balance(),
        balances.amount_due,
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_minimums_follow_currency_precision() {
        assert_eq!(
            provider_minimum_charge(PaymentProvider::Stripe, "eur"),
            Some(50)
        );
        assert_eq!(
            provider_minimum_charge(PaymentProvider::Stripe, "GBP"),
            Some(30)
        );
        assert_eq!(
            provider_minimum_charge(PaymentProvider::Stripe, "JPY"),
            Some(50)
        );
        assert_eq!(
            provider_minimum_charge(PaymentProvider::Gocardless, "EUR"),
            Some(100)
        );
        assert_eq!(
            provider_minimum_charge(PaymentProvider::Stancer, "EUR"),
            Some(50)
        );
        assert_eq!(
            provider_minimum_charge(PaymentProvider::Stancer, "USD"),
            None
        );
        assert_eq!(provider_minimum_charge(PaymentProvider::Mock, "EUR"), None);
    }

    #[test]
    fn the_highest_minimum_applies_converted_into_the_invoice_currency() {
        let stripe = [PaymentProvider::Stripe];
        assert_eq!(
            min_charge_amount(Some(100), &stripe, "EUR", "EUR", None),
            100
        );
        assert_eq!(
            min_charge_amount(Some(20), &stripe, "EUR", "EUR", None),
            50,
            "the entity floor never lowers the provider's"
        );
        assert_eq!(min_charge_amount(None, &[], "EUR", "EUR", None), 0);

        let usd_per_eur =
            std::collections::BTreeMap::from([("EUR".to_owned(), 0.8), ("GBP".to_owned(), 0.75)]);
        let to_usd = FxTable::from_usd_rates("USD", &usd_per_eur);
        assert_eq!(
            min_charge_amount(Some(100), &stripe, "EUR", "USD", Some(&to_usd)),
            125,
            "1.00 EUR is 1.25 USD"
        );
        assert_eq!(
            min_charge_amount(None, &stripe, "EUR", "USD", Some(&to_usd)),
            63,
            "Stripe's 0.50 EUR settlement minimum, above its 0.50 USD one"
        );
        assert_eq!(
            min_charge_amount(Some(100), &stripe, "EUR", "USD", None),
            50,
            "without rates only the invoice currency's minimum is known"
        );
        let to_gbp = FxTable::from_usd_rates("GBP", &usd_per_eur);
        assert_eq!(
            min_charge_amount(None, &stripe, "EUR", "GBP", Some(&to_gbp)),
            47
        );
    }

    #[test]
    fn a_small_amount_due_is_carried_only_when_a_later_invoice_takes_it() {
        assert_eq!(
            CarriedBalances::decide(30, 30, 0, 50, true),
            CarriedBalances {
                previous_balance: 0,
                carried_forward: 30,
                amount_due: 0
            }
        );
        assert_eq!(CarriedBalances::decide(30, 30, 0, 50, false).amount_due, 30);
        assert_eq!(
            CarriedBalances::decide(50, 50, 0, 50, true).carried_forward,
            0
        );
    }

    #[test]
    fn a_carried_debt_is_collected_once_the_amount_reaches_the_minimum() {
        assert_eq!(
            CarriedBalances::decide(1000, 1000, 30, 50, true),
            CarriedBalances {
                previous_balance: 30,
                carried_forward: 0,
                amount_due: 1030
            }
        );
        assert_eq!(
            CarriedBalances::decide(10, 10, 30, 50, true),
            CarriedBalances {
                previous_balance: 30,
                carried_forward: 40,
                amount_due: 0
            }
        );
    }

    #[test]
    fn a_negative_total_is_carried_as_credit_and_settles_a_carried_debt() {
        assert_eq!(
            CarriedBalances::decide(-500, 0, 0, 50, true),
            CarriedBalances {
                previous_balance: 0,
                carried_forward: -500,
                amount_due: 0
            }
        );
        assert_eq!(
            CarriedBalances::decide(-500, 0, 30, 50, true),
            CarriedBalances {
                previous_balance: 30,
                carried_forward: -470,
                amount_due: 0
            }
        );
        assert_eq!(
            CarriedBalances::decide(-20, 0, 30, 50, true),
            CarriedBalances {
                previous_balance: 30,
                carried_forward: 10,
                amount_due: 0
            }
        );
    }
}
