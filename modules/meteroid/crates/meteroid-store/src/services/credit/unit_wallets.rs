//! Credit unit wallets are funded by credit packs, included credit and grants, and sold directly
//! (top-ups, automatic top-ups, credit purchases) at the unit's price in the customer's currency.
//! A sale is taxed when it is made, under the unit's product, like a pack granting the unit. Like
//! every credit purchase, the price excludes tax: tax-inclusive pricing is a plan version's.
use super::purchase::credit_purchase_line;
use crate::StoreResult;
use crate::constants::Currencies;
use crate::domain::credit::{TopUpPricing, TopUpWalletQuote};
use crate::domain::credit_exhausted::{CreditExhaustedRule, UsageAtZero};
use crate::domain::credit_units::{
    CreditUnit, CreditUnitRef, credits_to_money_minor, overage_rate,
};
use crate::domain::prices::{Pricing, validate_credit_pack, validate_credit_rollover};
use crate::domain::{CreditPurchaseLineName, Customer, LineItem};
use crate::errors::StoreError;
use crate::repositories::invoicing_entities::InvoicingEntityInterface;
use crate::services::credit::{invalid, precondition};
use crate::services::{Services, ServicesEdge};
use crate::store::PgConn;
use common_domain::ids::{
    BaseId, CreditUnitId, CreditUnitPriceId, CustomerId, PlanVersionId, ProductId, TenantId,
    WalletId,
};
use diesel_models::credit_units::{CreditUnitPriceRow, CreditUnitRow};
use diesel_models::customer_credit::{CustomerCreditAccountRow, CustomerCreditOperationRow};
use diesel_models::customers::CustomerRow;
use diesel_models::enums::CreditExhaustedRuleEnum;
use diesel_models::plan_versions::PlanVersionRow;
use diesel_models::products::ProductRow;
use error_stack::Report;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

/// Where a top-up's operation pins the terms of a unit sale.
pub const UNIT_SALE_KEY: &str = "unit_sale";

/// Where the price of a unit sale comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum UnitPriceSource {
    List,
    /// The price the customer agreed for the wallet.
    Customer,
    /// A price given for this one purchase.
    Override,
}

impl UnitPriceSource {
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::List => "LIST",
            Self::Customer => "CUSTOMER",
            Self::Override => "OVERRIDE",
        }
    }
}

/// The terms of selling credits of a unit: the price in the customer's currency (a purchase's
/// own price, else the customer's agreed price, else the list price) and the product the sale is
/// booked under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitSale {
    pub price_id: CreditUnitPriceId,
    pub list_price: Decimal,
    /// Major units of `currency` per credit.
    pub unit_price: Decimal,
    pub source: UnitPriceSource,
    pub currency: String,
    pub currency_precision: u8,
    pub product_id: ProductId,
    pub unit: CreditUnitRef,
    pub plural_name: String,
}

/// What an operation pins: enough to charge exactly what was quoted whatever the price becomes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PinnedSale {
    price_id: CreditUnitPriceId,
    currency: String,
    unit_price: String,
    product_id: ProductId,
    net_minor: i64,
    source: UnitPriceSource,
    /// The tax and the card charge, once quoted (at creation for a self-service top-up, when the
    /// document is drafted for an automatic one).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tax_minor: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    total_minor: Option<i64>,
}

impl UnitSale {
    pub fn net_minor(&self, credits_minor: i64) -> StoreResult<i64> {
        credits_to_money_minor(
            credits_minor,
            self.unit.precision,
            self.unit_price,
            self.currency_precision,
        )
    }

    pub fn pricing(&self) -> TopUpPricing {
        TopUpPricing {
            unit_precision: self.unit.precision,
            unit_price: self.unit_price,
            currency_precision: self.currency_precision,
        }
    }

    fn description(&self, credits: Decimal, expires_in_months: Option<u32>) -> String {
        let mut description = format!("{} {}", group_thousands(credits), self.plural_name);
        if let Some(months) = expires_in_months {
            description.push_str(&format!(", valid for {months} months after payment"));
        }
        description
    }

    /// The single line of the sale's invoice: a credit line granting the unit, taxed at purchase.
    /// `quantity × unit_price` rounds to its amount exactly as `net_minor` does.
    pub(crate) fn line(
        &self,
        name: &str,
        credits_minor: i64,
        date: chrono::NaiveDate,
        expires_in_months: Option<u32>,
    ) -> StoreResult<LineItem> {
        let net = self.net_minor(credits_minor)?;
        let credits = Decimal::new(credits_minor, u32::from(self.unit.precision));
        Ok(LineItem {
            name: name.to_owned(),
            description: Some(self.description(credits, expires_in_months)),
            unit_price: Some(self.unit_price),
            quantity: Some(credits),
            credit_grant_amount: Some(credits_minor),
            credit_grant_unit: Some(self.unit.clone()),
            taxed_at_purchase: true,
            product_id: Some(self.product_id),
            ..credit_purchase_line(net, self.currency_precision, date, expires_in_months)
        })
    }

    /// The line of a sale whose charge was captured at `total_minor`, tax included: its document
    /// is drafted tax-inclusive, so the tax today's rules carve out of it never changes the total.
    pub(crate) fn line_at_total(
        &self,
        name: &str,
        credits_minor: i64,
        total_minor: i64,
        date: chrono::NaiveDate,
    ) -> StoreResult<LineItem> {
        let credits = Decimal::new(credits_minor, u32::from(self.unit.precision));
        let total = Decimal::new(total_minor, u32::from(self.currency_precision));
        Ok(LineItem {
            name: name.to_owned(),
            description: Some(self.description(credits, None)),
            unit_price: total.checked_div(credits),
            quantity: Some(credits),
            credit_grant_amount: Some(credits_minor),
            credit_grant_unit: Some(self.unit.clone()),
            taxed_at_purchase: true,
            product_id: Some(self.product_id),
            ..credit_purchase_line(total_minor, self.currency_precision, date, None)
        })
    }

    /// The operation metadata entry that pins this sale for `credits_minor`, with its quoted
    /// tax and charge when known.
    pub fn pin(&self, credits_minor: i64, quote: Option<&TopUpWalletQuote>) -> StoreResult<Value> {
        serde_json::to_value(PinnedSale {
            price_id: self.price_id,
            currency: self.currency.clone(),
            unit_price: self.unit_price.normalize().to_string(),
            product_id: self.product_id,
            net_minor: self.net_minor(credits_minor)?,
            source: self.source,
            tax_minor: quote.map(|quote| quote.tax_minor),
            total_minor: quote.map(|quote| quote.charge_minor),
        })
        .map_err(|error| Report::new(StoreError::SerdeError("unit sale".into(), error)))
    }
}

/// Records the tax and charge of a pinned sale once its document priced them.
pub(crate) fn pin_charge(
    metadata: &mut Value,
    tax_minor: i64,
    total_minor: i64,
) -> StoreResult<()> {
    let pinned = metadata
        .get_mut(UNIT_SALE_KEY)
        .and_then(Value::as_object_mut)
        .ok_or_else(|| precondition("the top-up has no pinned credit price"))?;
    pinned.insert("tax_minor".to_owned(), Value::from(tax_minor));
    pinned.insert("total_minor".to_owned(), Value::from(total_minor));
    Ok(())
}

/// The card charge, tax included, a sale pinned once it was quoted.
pub(crate) fn pinned_total_minor(metadata: &Value) -> Option<i64> {
    metadata
        .get(UNIT_SALE_KEY)
        .and_then(|pinned| pinned.get("total_minor"))
        .and_then(Value::as_i64)
}

/// The quote a sale pinned with its tax, when it pinned one.
pub(crate) fn pinned_quote(metadata: &Value) -> Option<TopUpWalletQuote> {
    let pinned: PinnedSale = serde_json::from_value(metadata.get(UNIT_SALE_KEY)?.clone()).ok()?;
    Some(TopUpWalletQuote {
        charge_minor: pinned.total_minor?,
        currency: pinned.currency,
        net_minor: pinned.net_minor,
        tax_minor: pinned.tax_minor?,
    })
}

/// What an operation charges or will charge: the card charge once known (always, for money
/// credit, the credit itself), otherwise the pinned price before tax. The caps count this.
pub fn charge_estimate_minor(operation: &CustomerCreditOperationRow) -> i64 {
    operation.expected_consideration_minor.unwrap_or_else(|| {
        operation
            .metadata
            .get(UNIT_SALE_KEY)
            .and_then(|pinned| pinned.get("net_minor"))
            .and_then(Value::as_i64)
            .unwrap_or(operation.amount_minor)
    })
}

/// The sale terms of `unit` in `currency`, or why it cannot be sold.
fn sale_of(
    unit: CreditUnit,
    price: Option<CreditUnitPriceRow>,
    product: Option<&ProductRow>,
    currency: &str,
    unit_price: Option<Decimal>,
    customer_price: Option<Decimal>,
) -> StoreResult<UnitSale> {
    if unit.archived_at.is_some() {
        return Err(precondition(format!(
            "{} are archived and can no longer be sold",
            unit.plural_name
        )));
    }
    let currency_precision = Currencies::resolve_currency_precision(currency)
        .ok_or_else(|| precondition(format!("unsupported currency {currency}")))?;
    let product_id = unit.product_id.ok_or_else(|| {
        precondition(format!(
            "{} cannot be sold until a product is set for them on the credit unit",
            unit.plural_name
        ))
    })?;
    match product {
        Some(product) if product.id == product_id && product.archived_at.is_none() => {}
        _ => {
            return Err(precondition(format!(
                "{} cannot be sold: their product is archived",
                unit.plural_name
            )));
        }
    }
    let price = price.ok_or_else(|| {
        precondition(format!(
            "{} have no selling price in {currency}: add one to the credit unit",
            unit.plural_name
        ))
    })?;
    let list_price = price.unit_price.normalize();
    let (unit_price, source) = match (unit_price, customer_price) {
        (Some(price), _) => (
            crate::domain::credit_units::validate_credit_unit_price(price)?,
            UnitPriceSource::Override,
        ),
        (None, Some(price)) => (price.normalize(), UnitPriceSource::Customer),
        (None, None) => (list_price, UnitPriceSource::List),
    };
    Ok(UnitSale {
        price_id: price.id,
        list_price,
        unit_price,
        source,
        currency: currency.to_owned(),
        currency_precision,
        product_id,
        unit: unit.reference(),
        plural_name: unit.plural_name,
    })
}

/// The customer's agreed price per credit of `account` when it is set in `currency`; a price
/// agreed in another currency does not apply.
fn customer_price(account: &CustomerCreditAccountRow, currency: &str) -> Option<Decimal> {
    account
        .unit_price
        .filter(|_| account.unit_price_currency.as_deref() == Some(currency))
}

/// What prices the credits a customer's usage leaves uncovered, in one invoice currency.
pub(crate) struct OverageRates {
    tenant_id: TenantId,
    currency: String,
    accounts: HashMap<CreditUnitId, CustomerCreditAccountRow>,
    list: HashMap<CreditUnitId, Decimal>,
    rules: HashMap<PlanVersionId, CreditExhaustedRule>,
    warned: HashSet<CreditUnitId>,
}

impl OverageRates {
    /// `plan_version_ids` are the versions the priced subscriptions bill, whatever their status.
    pub(crate) async fn load(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        currency: &str,
        unit_ids: &[CreditUnitId],
        plan_version_ids: &[PlanVersionId],
    ) -> StoreResult<Self> {
        let currency = currency.trim().to_ascii_uppercase();
        let accounts = CustomerCreditAccountRow::list_for_customer(conn, tenant_id, customer_id)
            .await?
            .into_iter()
            .filter_map(|account| account.credit_unit_id.map(|unit_id| (unit_id, account)))
            .collect();
        let list = CreditUnitPriceRow::list_by_units(conn, tenant_id, unit_ids, false)
            .await?
            .into_iter()
            .filter(|price| price.currency == currency)
            .map(|price| (price.credit_unit_id, price.unit_price))
            .collect();
        let rules = PlanVersionRow::list_by_ids_and_tenant_id(conn, plan_version_ids, tenant_id)
            .await?
            .into_iter()
            .map(|version| {
                let rule = version
                    .credit_exhausted_rule
                    .unwrap_or(CreditExhaustedRuleEnum::Invoice);
                let rule = CreditExhaustedRule::from_row(rule, &version.credit_exhausted_caps);
                (version.id, rule)
            })
            .collect();
        Ok(Self {
            tenant_id,
            currency,
            accounts,
            list,
            rules,
            warned: HashSet::new(),
        })
    }

    /// The rate credits of `unit_id` that a subscription billing `plan_version_id` uses at
    /// `price_rate` (the price's own rate) are billed at beyond the balance (`overage_rate`),
    /// invoiced past zero exactly where the access check lets usage through.
    pub(crate) fn rate(
        &mut self,
        unit_id: CreditUnitId,
        code: &str,
        plan_version_id: PlanVersionId,
        price_rate: Decimal,
    ) -> Decimal {
        let account = self.accounts.get(&unit_id);
        let usage_at_zero = account.map_or(UsageAtZero::FollowPlan, |a| a.usage_at_zero.into());
        let invoiced = usage_at_zero.allowance(self.rules.get(&plan_version_id), code) != Some(0);
        let rate = overage_rate(
            price_rate,
            invoiced,
            account.and_then(|account| customer_price(account, &self.currency)),
            self.list.get(&unit_id).copied(),
        );
        if invoiced && rate.is_zero() && self.warned.insert(unit_id) {
            log::warn!(
                "credits of unit {unit_id} used past zero are invoiced at no money (tenant {}, \
                 {}): the unit has no value in that currency",
                self.tenant_id,
                self.currency
            );
        }
        rate
    }
}

/// The sale terms of `account` in `currency`; None for a wallet held in money. `unit_price`
/// overrides the customer's agreed price and the list price (an operator's negotiated price for
/// this purchase); the list price must still exist.
pub async fn resolve_unit_sale(
    conn: &mut PgConn,
    account: &CustomerCreditAccountRow,
    currency: &str,
    unit_price: Option<Decimal>,
) -> StoreResult<Option<UnitSale>> {
    let Some(unit_id) = account.credit_unit_id else {
        return Ok(None);
    };
    let unit = load_unit(conn, account.tenant_id, unit_id).await?;
    let currency = currency.trim().to_ascii_uppercase();
    let price =
        CreditUnitPriceRow::find_active(conn, account.tenant_id, unit_id, &currency).await?;
    let product = match unit.product_id {
        Some(id) => ProductRow::list_by_ids(conn, &[id], account.tenant_id)
            .await?
            .into_iter()
            .next(),
        None => None,
    };
    let agreed = customer_price(account, &currency);
    sale_of(unit, price, product.as_ref(), &currency, unit_price, agreed).map(Some)
}

/// The sale terms (the customer's agreed price, else the list price) of each credit unit wallet
/// of `accounts` that can be sold in `currency` now, by wallet, in a fixed number of queries.
pub(crate) async fn sellable_unit_prices(
    conn: &mut PgConn,
    tenant_id: TenantId,
    accounts: &[CustomerCreditAccountRow],
    currency: &str,
) -> StoreResult<HashMap<WalletId, UnitSale>> {
    let mut unit_ids: Vec<CreditUnitId> = accounts
        .iter()
        .filter_map(|account| account.credit_unit_id)
        .collect();
    unit_ids.sort_by_key(|id| id.as_uuid());
    unit_ids.dedup();
    if unit_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let currency = currency.trim().to_ascii_uppercase();
    let units: HashMap<CreditUnitId, CreditUnit> =
        CreditUnitRow::list_by_ids(conn, tenant_id, &unit_ids)
            .await?
            .into_iter()
            .map(|row| CreditUnit::try_from(row).map(|unit| (unit.id, unit)))
            .collect::<StoreResult<_>>()?;
    let prices: HashMap<CreditUnitId, CreditUnitPriceRow> =
        CreditUnitPriceRow::list_by_units(conn, tenant_id, &unit_ids, false)
            .await?
            .into_iter()
            .filter(|price| price.currency == currency)
            .map(|price| (price.credit_unit_id, price))
            .collect();
    let product_ids: Vec<ProductId> = units.values().filter_map(|unit| unit.product_id).collect();
    let products: HashMap<ProductId, ProductRow> = if product_ids.is_empty() {
        HashMap::new()
    } else {
        ProductRow::list_by_ids(conn, &product_ids, tenant_id)
            .await?
            .into_iter()
            .map(|product| (product.id, product))
            .collect()
    };
    let mut sales = HashMap::new();
    for account in accounts {
        let Some(unit) = account.credit_unit_id.and_then(|id| units.get(&id)) else {
            continue;
        };
        let product = unit.product_id.and_then(|id| products.get(&id));
        let price = prices.get(&unit.id).cloned();
        let agreed = customer_price(account, &currency);
        if let Ok(sale) = sale_of(unit.clone(), price, product, &currency, None, agreed) {
            sales.insert(account.id, sale);
        }
    }
    Ok(sales)
}

/// The sale an operation pinned; None when it pinned none (a money wallet).
pub async fn pinned_unit_sale(
    conn: &mut PgConn,
    account: &CustomerCreditAccountRow,
    metadata: &Value,
) -> StoreResult<Option<UnitSale>> {
    let (Some(unit_id), Some(pinned)) = (account.credit_unit_id, metadata.get(UNIT_SALE_KEY))
    else {
        return Ok(None);
    };
    let pinned: PinnedSale = serde_json::from_value(pinned.clone())
        .map_err(|error| Report::new(StoreError::SerdeError("unit sale".into(), error)))?;
    let unit = load_unit(conn, account.tenant_id, unit_id).await?;
    let price = CreditUnitPriceRow::find_by_id(conn, account.tenant_id, pinned.price_id)
        .await?
        .filter(|price| price.credit_unit_id == unit_id && price.currency == pinned.currency)
        .ok_or_else(|| precondition("the pinned credit price does not belong to this wallet"))?;
    let unit_price = pinned
        .unit_price
        .parse::<Decimal>()
        .map_err(|_| precondition("the pinned credit price is malformed"))?;
    Ok(Some(UnitSale {
        price_id: price.id,
        list_price: price.unit_price.normalize(),
        unit_price,
        source: pinned.source,
        currency_precision: Currencies::resolve_currency_precision(&pinned.currency)
            .ok_or_else(|| precondition(format!("unsupported currency {}", pinned.currency)))?,
        currency: pinned.currency,
        product_id: pinned.product_id,
        unit: unit.reference(),
        plural_name: unit.plural_name,
    }))
}

async fn load_unit(
    conn: &mut PgConn,
    tenant_id: TenantId,
    unit_id: CreditUnitId,
) -> StoreResult<CreditUnit> {
    CreditUnitRow::find_by_id(conn, tenant_id, unit_id)
        .await?
        .ok_or_else(|| precondition(format!("credit unit {unit_id} is missing")))?
        .try_into()
}

/// `50000` as `50,000`; the fraction, if any, is kept as it is.
fn group_thousands(value: Decimal) -> String {
    let text = value.normalize().to_string();
    let (whole, fraction) = text
        .split_once('.')
        .map_or((text.as_str(), None), |(w, f)| (w, Some(f)));
    let digits = whole.trim_start_matches('-');
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    match fraction {
        Some(fraction) => format!("{grouped}.{fraction}"),
        None => grouped,
    }
}

/// What selling credits would charge, with the list price and where the applied price comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreditSaleQuote {
    pub quote: TopUpWalletQuote,
    /// Credit unit wallets only.
    pub list_unit_price: Option<Decimal>,
    pub unit_price: Option<Decimal>,
    pub unit_price_source: Option<UnitPriceSource>,
}

impl ServicesEdge {
    /// The price of selling `amount_minor` into a wallet, by invoice or by card; charges nothing.
    /// Money credit charges its own amount untaxed; credits of a unit their price plus tax.
    pub async fn quote_credit_sale(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        wallet_id: WalletId,
        amount_minor: i64,
        unit_price: Option<Decimal>,
    ) -> StoreResult<CreditSaleQuote> {
        if amount_minor <= 0 {
            return Err(invalid("credit is sold in positive amounts"));
        }
        let mut conn = self.store.get_conn().await?;
        let account = CustomerCreditAccountRow::find_by_id(&mut conn, tenant_id, wallet_id)
            .await?
            .filter(|account| account.customer_id == customer_id)
            .ok_or_else(|| Report::new(StoreError::ValueNotFound("wallet".to_owned())))?;
        let customer: Customer = CustomerRow::find_by_id(&mut conn, &customer_id, &tenant_id)
            .await?
            .try_into()?;
        match resolve_unit_sale(&mut conn, &account, &customer.currency, unit_price).await? {
            Some(sale) => Ok(CreditSaleQuote {
                quote: self
                    .services
                    .quote_unit_sale_tx(&mut conn, &customer, &sale, amount_minor)
                    .await?,
                list_unit_price: Some(sale.list_price),
                unit_price: Some(sale.unit_price),
                unit_price_source: Some(sale.source),
            }),
            None if unit_price.is_some() => {
                Err(invalid("a unit price applies to credit unit wallets only"))
            }
            None => Ok(CreditSaleQuote {
                quote: TopUpWalletQuote {
                    charge_minor: amount_minor,
                    currency: account.currency.clone(),
                    net_minor: amount_minor,
                    tax_minor: 0,
                },
                list_unit_price: None,
                unit_price: None,
                unit_price_source: None,
            }),
        }
    }

    /// The price each credit unit wallet of the customer sells at in their currency (their agreed
    /// price, else the list price), by wallet; a unit that cannot be sold now (no list price in
    /// that currency, no product) is absent.
    pub async fn sellable_unit_prices(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
    ) -> StoreResult<HashMap<WalletId, UnitSale>> {
        let mut conn = self.store.get_conn().await?;
        let customer = CustomerRow::find_by_id(&mut conn, &customer_id, &tenant_id).await?;
        let accounts =
            CustomerCreditAccountRow::list_for_customer(&mut conn, tenant_id, customer_id).await?;
        sellable_unit_prices(&mut conn, tenant_id, &accounts, &customer.currency).await
    }
}

impl Services {
    /// The net, tax and total of a sale, computed as its invoice will be.
    pub(crate) async fn quote_unit_sale_tx(
        &self,
        conn: &mut PgConn,
        customer: &Customer,
        sale: &UnitSale,
        credits_minor: i64,
    ) -> StoreResult<TopUpWalletQuote> {
        let date = common_domain::clock::now().date_naive();
        let line = sale.line(
            CreditPurchaseLineName::Purchase.stored_name(),
            credits_minor,
            date,
            None,
        )?;
        if line.amount_subtotal <= 0 {
            return Err(invalid(format!(
                "these credits are worth less than the smallest amount of {}",
                sale.currency
            )));
        }
        let entity = self
            .store
            .get_invoicing_entity_with_conn(
                conn,
                customer.tenant_id,
                Some(customer.invoicing_entity_id),
            )
            .await?;
        let content = self
            .compute_oneoff_invoice(
                conn,
                &date,
                vec![line],
                &entity,
                customer,
                sale.currency.clone(),
                None,
                None,
                false,
            )
            .await?;
        Ok(TopUpWalletQuote {
            charge_minor: content.total,
            currency: sale.currency.clone(),
            net_minor: content.subtotal,
            tax_minor: content.tax_amount,
        })
    }
}

/// Refuses a credit pack granting a unit the tenant does not have or archived, or an amount (or
/// a rollover cap) finer than that unit's precision.
pub(crate) async fn ensure_credit_packs_usable<'a>(
    conn: &mut PgConn,
    tenant_id: TenantId,
    currency: &str,
    pricings: impl IntoIterator<Item = &'a Pricing>,
) -> StoreResult<()> {
    let packs: Vec<_> = pricings
        .into_iter()
        .filter(|pricing| {
            matches!(
                pricing,
                Pricing::Credit {
                    credit_unit_id: Some(_),
                    ..
                }
            )
        })
        .collect();
    if packs.is_empty() {
        return Ok(());
    }
    let mut ids: Vec<CreditUnitId> = packs
        .iter()
        .filter_map(|pricing| match pricing {
            Pricing::Credit { credit_unit_id, .. } => *credit_unit_id,
            _ => None,
        })
        .collect();
    ids.sort_by_key(|id| id.as_uuid());
    ids.dedup();
    let units: HashMap<CreditUnitId, CreditUnit> =
        CreditUnitRow::list_by_ids(conn, tenant_id, &ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .map(|row| CreditUnit::try_from(row).map(|unit| (unit.id, unit)))
            .collect::<StoreResult<_>>()?;
    for pricing in packs {
        let Pricing::Credit {
            unit_price,
            quantity,
            credit_amount,
            expiry,
            credit_unit_id: Some(unit_id),
            rollover,
            ..
        } = pricing
        else {
            continue;
        };
        let unit = units
            .get(unit_id)
            .ok_or_else(|| invalid(format!("credit unit {unit_id} not found")))?;
        unit.ensure_active_for_new_pricing()?;
        validate_pack_in_unit(
            *unit_price,
            *quantity,
            *credit_amount,
            *expiry,
            rollover.as_ref(),
            currency,
            unit.precision,
        )?;
    }
    Ok(())
}

fn validate_pack_in_unit(
    unit_price: rust_decimal::Decimal,
    quantity: u32,
    credit_amount: rust_decimal::Decimal,
    expiry: Option<crate::domain::price_components::CreditExpiry>,
    rollover: Option<&crate::domain::price_components::CreditRollover>,
    currency: &str,
    precision: u8,
) -> StoreResult<()> {
    let precision = validate_credit_pack(
        unit_price,
        quantity,
        credit_amount,
        expiry,
        currency,
        Some(precision),
    )
    .map_err(Report::new)?;
    validate_credit_rollover(unit_price, expiry, rollover, precision).map_err(Report::new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::price_components::{CreditExpiry, CreditRollover};
    use rust_decimal_macros::dec;

    fn sale(unit_precision: u8, unit_price: Decimal, currency: &str) -> UnitSale {
        UnitSale {
            price_id: CreditUnitPriceId::new(),
            list_price: unit_price,
            unit_price,
            source: UnitPriceSource::List,
            currency: currency.to_owned(),
            currency_precision: Currencies::resolve_currency_precision(currency).unwrap(),
            product_id: ProductId::new(),
            unit: CreditUnitRef {
                unit_id: CreditUnitId::new(),
                code: "HALYARD".to_owned(),
                precision: unit_precision,
                product_id: None,
            },
            plural_name: "Halyard credits".to_owned(),
        }
    }

    #[test]
    fn credits_are_priced_once_rounding_halves_away_from_zero() {
        // 50,000 credits at $0.001: $50.00.
        assert_eq!(
            sale(0, dec!(0.001), "USD").net_minor(50_000).unwrap(),
            5_000
        );
        // 12,345 credits at $0.0013 = $16.0485, rounded once to $16.05.
        assert_eq!(
            sale(0, dec!(0.0013), "USD").net_minor(12_345).unwrap(),
            1_605
        );
        // 1.25 credits (precision 2) at $0.10 = $0.125: a half rounds up to $0.13.
        assert_eq!(sale(2, dec!(0.10), "USD").net_minor(125).unwrap(), 13);
        // Precision 6 credits finer than the currency: 0.000500 credit at 1 EUR is 0.0005 EUR.
        assert_eq!(sale(6, dec!(1), "EUR").net_minor(500).unwrap(), 0);
        assert_eq!(sale(6, dec!(1), "EUR").net_minor(5_000).unwrap(), 1);
        // A zero-decimal currency: 333 credits at 1.5 JPY = 499.5 JPY, rounded to 500.
        assert_eq!(sale(0, dec!(1.5), "JPY").net_minor(333).unwrap(), 500);
        // A three-decimal currency: 7 credits at 0.0005 BHD = 0.0035 BHD, rounded to 0.004.
        assert_eq!(sale(0, dec!(0.0005), "BHD").net_minor(7).unwrap(), 4);
        assert!(sale(0, dec!(0.001), "USD").net_minor(-1).is_err());
    }

    #[test]
    fn the_sale_line_reconciles_quantity_and_unit_price_to_its_amount() {
        use common_utils::decimals::ToSubunit;
        let date = chrono::NaiveDate::from_ymd_opt(2026, 10, 8).unwrap();
        for (precision, price, currency, credits) in [
            (0_u8, dec!(0.001), "USD", 50_000_i64),
            (0, dec!(0.0013), "USD", 12_345),
            (2, dec!(0.10), "USD", 125),
            (3, dec!(0.07), "EUR", 1_001),
            (0, dec!(1.5), "JPY", 333),
        ] {
            let sale = sale(precision, price, currency);
            let line = sale.line("Purchase credits", credits, date, None).unwrap();
            let derived = (line.quantity.unwrap() * line.unit_price.unwrap())
                .to_subunit_opt(sale.currency_precision)
                .unwrap();
            assert_eq!(
                derived, line.amount_subtotal,
                "{credits} at {price} {currency}"
            );
            assert_eq!(line.credit_grant_amount, Some(credits));
            assert!(line.taxed_at_purchase);
            assert_eq!(line.product_id, Some(sale.product_id));
            assert_eq!(
                line.credit_grant_unit.as_ref().map(|u| u.precision),
                Some(precision)
            );
        }
        let line = sale(0, dec!(0.001), "USD")
            .line("Purchase credits", 50_000, date, None)
            .unwrap();
        assert_eq!(line.description.as_deref(), Some("50,000 Halyard credits"));
    }

    #[test]
    fn an_agreed_price_applies_only_in_its_own_currency() {
        let account = CustomerCreditAccountRow {
            id: WalletId::new(),
            tenant_id: TenantId::new(),
            customer_id: CustomerId::new(),
            currency: "HALYARD".to_owned(),
            created_at: common_domain::clock::now(),
            low_balance_thresholds_minor: vec![],
            credit_unit_id: Some(CreditUnitId::new()),
            unit_price: Some(dec!(0.0008)),
            unit_price_currency: Some("EUR".to_owned()),
            ledger_version: 0,
            usage_at_zero: diesel_models::enums::UsageAtZeroEnum::FollowPlan,
        };
        assert_eq!(customer_price(&account, "EUR"), Some(dec!(0.0008)));
        assert_eq!(customer_price(&account, "USD"), None);
    }

    #[test]
    fn a_pin_records_where_its_price_came_from() {
        let pin = sale(0, dec!(0.001), "USD").pin(1_000, None).unwrap();
        assert_eq!(pin["source"], "LIST");
    }

    #[test]
    fn a_pin_carries_the_pre_tax_money_the_caps_count() {
        let sale = sale(0, dec!(0.001), "USD");
        let pin = sale.pin(50_000, None).unwrap();
        assert_eq!(pin["net_minor"], 5_000);
        assert_eq!(pin["unit_price"], "0.001");
    }

    #[test]
    fn a_pack_grants_whole_minor_units_of_its_unit_not_of_the_currency() {
        // 0.001 credit is finer than a USD cent but whole in a unit of precision 3.
        assert!(validate_pack_in_unit(dec!(10), 1, dec!(0.001), None, None, "USD", 3).is_ok());
        assert!(validate_pack_in_unit(dec!(10), 1, dec!(0.001), None, None, "USD", 2).is_err());
        assert!(validate_pack_in_unit(dec!(10), 1, dec!(5000), None, None, "USD", 0).is_ok());
        assert!(validate_pack_in_unit(dec!(10), 1, dec!(0.5), None, None, "USD", 0).is_err());
    }

    #[test]
    fn a_rollover_cap_is_whole_minor_units_of_the_unit() {
        let monthly = Some(CreditExpiry::Month(1));
        let cap = |max: rust_decimal::Decimal| CreditRollover {
            max_amount: Some(max),
            expiry: CreditExpiry::Month(1),
        };
        assert!(
            validate_pack_in_unit(
                dec!(0),
                1,
                dec!(500),
                monthly,
                Some(&cap(dec!(200))),
                "EUR",
                0
            )
            .is_ok()
        );
        assert!(
            validate_pack_in_unit(
                dec!(0),
                1,
                dec!(500),
                monthly,
                Some(&cap(dec!(0.5))),
                "EUR",
                0
            )
            .is_err()
        );
    }
}
