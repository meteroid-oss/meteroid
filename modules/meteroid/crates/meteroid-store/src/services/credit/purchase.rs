//! Selling invoice credit: a one-off invoice with a single `Credit` line; paying it funds a paid
//! grant exactly once.
use crate::StoreResult;
use crate::constants::Currencies;
use crate::domain::credit::CreditWallet;
use crate::domain::entity_activity::Actor;
use crate::domain::enums::InvoiceStatusEnum;
use crate::domain::price_components::CreditExpiry;
use crate::domain::{ChargeType, CreditPurchaseLineName, Customer, DetailedInvoice, LineItem};
use crate::errors::StoreError;
use crate::repositories::InvoiceInterface;
use crate::repositories::credit::CustomerCreditWalletsInterface;
use crate::repositories::credit::customer_credit::source;
use crate::services::credit::invalid;
use crate::services::{InvoiceDelivery, IssueAt, Services, ServicesEdge};
use crate::store::PgConn;
use crate::utils::local_id::{IdType, LocalId};
use common_domain::ids::{BaseId, CreditGrantId, CustomerId, TenantId, WalletId};
use diesel_models::customer_credit::{
    CreditPurchaseRow, CreditPurchaseRowNew, CustomerCreditAccountRow, CustomerCreditGrantRow,
};
use diesel_models::customers::CustomerRow;
use error_stack::Report;
use rust_decimal::Decimal;
use scoped_futures::ScopedFutureExt;

/// Longest commitment a credit purchase can carry: ten years.
const MAX_EXPIRY_MONTHS: u32 = 120;

pub(crate) struct CreditPurchase {
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub id: uuid::Uuid,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub created_by: Option<uuid::Uuid>,
    /// Credit sold, in minor units of the wallet: money, or credits of `unit_sale`'s unit.
    pub amount_minor: i64,
    pub notes: Option<String>,
    pub expires_in_months: Option<u32>,
    /// A past moment the purchase is recorded and its invoice dated at; None is now.
    pub at: Option<chrono::DateTime<chrono::Utc>>,
    pub unit_sale: Option<super::unit_wallets::UnitSale>,
    /// The operator's negotiated price per credit, as requested.
    pub negotiated_unit_price: Option<Decimal>,
}

/// Paid invoice credit sold into a wallet: the invoice is issued at once and the grant is funded
/// when it is paid.
#[derive(Debug, Clone)]
pub struct SellCustomerCredit {
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub wallet_id: WalletId,
    /// In minor units of the wallet: money, or credits for a credit unit wallet.
    pub amount_minor: i64,
    /// Months the credit stays spendable once the invoice is paid.
    pub expires_in_months: Option<u32>,
    pub note: Option<String>,
    pub actor: Actor,
    /// A credit unit wallet only: the negotiated price of one credit for this sale, instead of the
    /// customer's agreed price or the list price.
    pub unit_price: Option<Decimal>,
    /// A credit unit wallet only: the price per credit the operator was shown. A sale that would
    /// charge another one is refused before its invoice is issued.
    pub expected_unit_price: Option<Decimal>,
}

#[derive(Debug, Clone)]
pub struct SoldCustomerCredit {
    pub wallet: CreditWallet,
    pub invoice: DetailedInvoice,
    /// Set once the invoice is paid.
    pub grant_id: Option<CreditGrantId>,
}

impl Services {
    /// Issues the draft purchase invoice in the customer's currency. `req.id` identifies the
    /// purchase, so a caller can replay it.
    pub(crate) async fn record_credit_purchase(
        &self,
        conn: &mut PgConn,
        req: CreditPurchase,
    ) -> StoreResult<DetailedInvoice> {
        let recorded_at = req.at.unwrap_or_else(common_domain::clock::now);
        if req.at.is_some_and(|at| at > common_domain::clock::now()) {
            return Err(invalid("credit cannot be purchased at a future moment"));
        }
        if req.expires_at.is_some_and(|at| at <= recorded_at) {
            return Err(invalid(match req.at {
                None => "credit expiry must be in the future",
                Some(_) => "credit expiry must be after the purchase",
            }));
        }
        if req.expires_at.is_some() && req.expires_in_months.is_some() {
            return Err(invalid("choose absolute or relative credit expiry"));
        }
        if req.amount_minor <= 0 {
            return Err(invalid("a credit purchase requires a positive charge"));
        }
        if req
            .expires_in_months
            .is_some_and(|months| !(1..=MAX_EXPIRY_MONTHS).contains(&months))
        {
            return Err(invalid(format!(
                "purchased credit expires between 1 and {MAX_EXPIRY_MONTHS} months after payment"
            )));
        }

        let invoice = self
            .store
            .transaction_with(conn, |conn| {
                async move {
                    let now = recorded_at.naive_utc();
                    // The customer first: the draft is one of its rows (lock order).
                    let lock = CustomerRow::lock(conn, req.tenant_id, req.customer_id).await?;
                    diesel_models::query::credit_inputs::bump_customer(conn, &lock).await?;

                    let customer: Customer =
                        CustomerRow::find_by_id(conn, &req.customer_id, &req.tenant_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)
                            .and_then(TryInto::try_into)?;

                    let currency = customer.currency.clone();
                    let precision =
                        Currencies::resolve_currency_precision(&currency).ok_or_else(|| {
                            invalid(format!("unsupported credit purchase currency {currency}"))
                        })?;
                    let line = match &req.unit_sale {
                        Some(sale) => sale.line(
                            CreditPurchaseLineName::Purchase.stored_name(),
                            req.amount_minor,
                            now.date(),
                            req.expires_in_months,
                        )?,
                        None => credit_purchase_line(
                            req.amount_minor,
                            precision,
                            now.date(),
                            req.expires_in_months,
                        ),
                    };
                    if line.amount_subtotal <= 0 {
                        return Err(invalid(format!(
                            "these credits are worth less than the smallest amount of {currency}"
                        )));
                    }
                    let money_minor = line.amount_subtotal;
                    let line_items = vec![line];

                    let invoice = self
                        .create_oneoff_draft_invoice(
                            conn,
                            req.tenant_id,
                            now.date(),
                            line_items,
                            &customer,
                            currency,
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                            // A credit purchase is an amount of money, not a plan price.
                            false,
                        )
                        .await?
                        .ok_or(
                            Report::new(StoreError::BillingError)
                                .attach("Failed to create one-off draft invoice"),
                        )?;

                    CreditPurchaseRow::insert(
                        conn,
                        &CreditPurchaseRowNew {
                            expires_at: req.expires_at,
                            id: req.id,
                            created_at: req.at.map(|at| at.naive_utc()),
                            amount_cents: money_minor,
                            amount_credits: req.unit_sale.is_some().then_some(req.amount_minor),
                            negotiated_unit_price: req.negotiated_unit_price,
                            unit_price_source: req
                                .unit_sale
                                .as_ref()
                                .map(|sale| sale.source.as_db_str().to_owned()),
                            note: req.notes.clone(),
                            invoice_id: invoice.id,
                            tenant_id: req.tenant_id,
                            customer_id: req.customer_id,
                            created_by: req.created_by,
                        },
                    )
                    .await?;

                    Ok(invoice)
                }
                .scope_boxed()
            })
            .await?;

        self.store
            .get_detailed_invoice_by_id(req.tenant_id, invoice.id)
            .await
    }
}

impl ServicesEdge {
    /// Sells paid credit into an invoice-credit wallet in the customer's currency. `purchase_id`
    /// identifies the sale: a retry finishes issuing the same invoice instead of selling again.
    pub async fn sell_customer_credit(
        &self,
        purchase_id: uuid::Uuid,
        command: SellCustomerCredit,
    ) -> StoreResult<SoldCustomerCredit> {
        self.sell_customer_credit_on(purchase_id, command, None, InvoiceDelivery::Normal)
            .await
    }

    /// `sell_customer_credit` as of `at`, a past moment: the purchase is recorded and its invoice
    /// issued then, and a chosen expiry must follow it.
    pub async fn sell_customer_credit_at(
        &self,
        purchase_id: uuid::Uuid,
        command: SellCustomerCredit,
        at: chrono::DateTime<chrono::Utc>,
        delivery: InvoiceDelivery,
    ) -> StoreResult<SoldCustomerCredit> {
        self.sell_customer_credit_on(purchase_id, command, Some(at), delivery)
            .await
    }

    async fn sell_customer_credit_on(
        &self,
        purchase_id: uuid::Uuid,
        command: SellCustomerCredit,
        at: Option<chrono::DateTime<chrono::Utc>>,
        delivery: InvoiceDelivery,
    ) -> StoreResult<SoldCustomerCredit> {
        let tenant_id = command.tenant_id;
        let mut conn = self.store.get_conn().await?;
        let account = CustomerCreditAccountRow::find_by_id(&mut conn, tenant_id, command.wallet_id)
            .await?
            .filter(|account| account.customer_id == command.customer_id)
            .ok_or_else(|| Report::new(StoreError::ValueNotFound("wallet".to_owned())))?;
        let customer = CustomerRow::find_by_id(&mut conn, &command.customer_id, &tenant_id).await?;
        if account.credit_unit_id.is_none()
            && (command.unit_price.is_some() || command.expected_unit_price.is_some())
        {
            return Err(invalid("a unit price applies to credit unit wallets only"));
        }
        if account.credit_unit_id.is_none()
            && !customer.currency.eq_ignore_ascii_case(&account.currency)
        {
            return Err(invalid(format!(
                "paid invoice credit is sold in the customer's currency {}",
                customer.currency
            )));
        }
        let invoice_id =
            match CreditPurchaseRow::find_by_id(&mut conn, tenant_id, purchase_id).await? {
                Some(purchase)
                    if purchase.customer_id == command.customer_id
                        && purchase.amount_credits.unwrap_or(purchase.amount_cents)
                            == command.amount_minor
                        && purchase
                            .negotiated_unit_price
                            .map(|price| price.normalize())
                            == command.unit_price.map(|price| price.normalize()) =>
                {
                    purchase.invoice_id
                }
                Some(_) => {
                    return Err(Report::new(StoreError::Conflict(
                        "this request already sold a different credit purchase".to_owned(),
                    )));
                }
                None => {
                    let unit_sale = super::unit_wallets::resolve_unit_sale(
                        &mut conn,
                        &account,
                        &customer.currency,
                        command.unit_price,
                    )
                    .await?;
                    ensure_expected_unit_price(unit_sale.as_ref(), command.expected_unit_price)?;
                    self.services
                        .record_credit_purchase(
                            &mut conn,
                            CreditPurchase {
                                id: purchase_id,
                                tenant_id,
                                customer_id: command.customer_id,
                                // The purchase row references a user; other actors leave it unset.
                                created_by: match &command.actor {
                                    Actor::User { id } => Some(id.as_uuid()),
                                    _ => None,
                                },
                                amount_minor: command.amount_minor,
                                notes: command.note,
                                expires_in_months: command.expires_in_months,
                                expires_at: command.expires_at,
                                at,
                                unit_sale,
                                negotiated_unit_price: command
                                    .unit_price
                                    .map(|price| price.normalize()),
                            },
                        )
                        .await?
                        .invoice
                        .id
                }
            };
        drop(conn);
        let invoice = self
            .store
            .get_detailed_invoice_by_id(tenant_id, invoice_id)
            .await?;
        // A replay after a failed finalization finishes issuing the invoice.
        let invoice = if invoice.invoice.status == InvoiceStatusEnum::Draft {
            let issue = at.map_or(IssueAt::Now, IssueAt::At);
            self.finalize_invoice_at(command.actor, invoice_id, tenant_id, issue, delivery)
                .await?
        } else {
            invoice
        };
        let mut conn = self.store.get_conn().await?;
        let grant_id = CustomerCreditGrantRow::find_by_source(
            &mut conn,
            tenant_id,
            source::CREDIT_PURCHASE,
            purchase_id,
        )
        .await?
        .map(|grant| grant.id);
        drop(conn);
        let wallet = self
            .store
            .customer_credit_wallet(tenant_id, command.wallet_id)
            .await?
            .ok_or_else(|| Report::new(StoreError::ValueNotFound("wallet".to_owned())))?;
        Ok(SoldCustomerCredit {
            wallet,
            invoice,
            grant_id,
        })
    }
}

/// Refuses a sale whose price is not the one its operator was shown.
pub(super) fn ensure_expected_unit_price(
    sale: Option<&super::unit_wallets::UnitSale>,
    expected: Option<Decimal>,
) -> StoreResult<()> {
    match (sale, expected) {
        (Some(sale), Some(expected)) if sale.unit_price.normalize() != expected.normalize() => Err(
            super::precondition(super::wallet_purchase::PRICE_CHANGED_REFRESH),
        ),
        _ => Ok(()),
    }
}

/// The single line of a credit purchase invoice. `unit_price` is a major-unit rate, so
/// `quantity * unit_price` reconciles to the minor-unit amount in every currency precision.
pub(super) fn credit_purchase_line(
    amount_minor: i64,
    precision: u8,
    date: chrono::NaiveDate,
    expires_in_months: Option<u32>,
) -> LineItem {
    LineItem {
        local_id: LocalId::generate_for(IdType::Other),
        name: CreditPurchaseLineName::Purchase.stored_name().into(),
        charge_type: Some(ChargeType::OneTime),
        amount_total: amount_minor,
        amount_subtotal: amount_minor,
        taxable_amount: amount_minor,
        tax_amount: 0,
        unit_price: Some(Decimal::new(amount_minor, u32::from(precision))),
        quantity: Some(Decimal::ONE),
        start_date: date,
        end_date: date,
        sub_lines: vec![],
        is_prorated: false,
        price_component_id: None,
        sub_component_id: None,
        sub_add_on_id: None,
        product_id: None,
        metric_id: None,
        description: expires_in_months
            .map(|months| format!("Credit valid for {months} months after payment")),
        tax_rate: Decimal::ZERO,
        tax_details: vec![],
        group_by_dimensions: None,
        kind: crate::domain::LineItemKind::Credit,
        credit_grant_amount: Some(amount_minor),
        credit_grant_expiry: expires_in_months.map(CreditExpiry::Month),
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

#[cfg(test)]
mod tests {
    use super::credit_purchase_line;
    use common_utils::decimals::ToSubunit;
    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;

    fn date() -> chrono::NaiveDate {
        chrono::NaiveDate::from_ymd_opt(2026, 7, 29).unwrap()
    }

    #[test]
    fn unit_price_is_a_major_unit_rate_not_the_cent_amount() {
        let line = credit_purchase_line(5_000, 2, date(), None);
        assert_eq!(line.unit_price, Some(dec!(50)));
        assert_eq!(line.quantity, Some(Decimal::ONE));
        assert_eq!(line.amount_subtotal, 5_000);
        assert_eq!(line.credit_grant_amount, Some(5_000));
        assert_eq!(line.kind, crate::domain::LineItemKind::Credit);
    }

    #[test]
    fn quantity_times_unit_price_reconciles_to_the_line_amount() {
        for (cents, precision) in [
            (5_000_i64, 2_u8),
            (1_024, 2),
            (1, 2),
            (5_000, 0),
            (5_000, 3),
            (999_999_999, 2),
        ] {
            let line = credit_purchase_line(cents, precision, date(), None);
            let derived = (line.quantity.unwrap() * line.unit_price.unwrap())
                .to_subunit_opt(precision)
                .unwrap();
            assert_eq!(
                derived, line.amount_subtotal,
                "{cents} at precision {precision}"
            );
            assert_eq!(derived, line.amount_total);
        }
    }

    #[test]
    fn a_commitment_expires_months_after_payment() {
        let line = credit_purchase_line(1_200_000, 2, date(), Some(12));
        assert_eq!(
            line.credit_grant_expiry,
            Some(crate::domain::price_components::CreditExpiry::Month(12))
        );
        assert!(
            credit_purchase_line(1, 2, date(), None)
                .credit_grant_expiry
                .is_none()
        );
    }
}
