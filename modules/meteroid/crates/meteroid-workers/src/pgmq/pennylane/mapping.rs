//! Meteroid documents as Pennylane imports. Pure: what Pennylane would refuse is refused here, with
//! a message the merchant can act on.

use chrono::NaiveDate;
use meteroid_store::domain::{
    Address, CreditNote, Customer, CustomerType, Invoice, LineItem, TaxBreakdownItem,
    TaxExemptionType, TaxKind,
};
use pennylane_client::customer_invoices::{
    Amount, CustomerInvoiceImport, ImputationDates, InvoiceLine, TransactionReference,
};
use pennylane_client::customers::{
    Address as PennylaneAddress, BillingLanguage, CompanyCustomer, CustomerUpsert,
    IndividualCustomer,
};
use pennylane_client::vat_rate::VatRate;
use rust_decimal::{Decimal, RoundingStrategy};

/// Why a document cannot be imported as it stands. Sending it again changes nothing: the
/// document, the customer or the Pennylane company has to change first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Unsyncable(pub String);

const MAX_REFERENCE_LEN: usize = 50;
const MAX_UNIT_PRICE_DECIMALS: u32 = 6;

pub(super) fn customer_upsert(customer: &Customer) -> CustomerUpsert {
    let billing_address = address(customer.billing_address.as_ref());
    let delivery_address = customer
        .shipping_address
        .as_ref()
        .filter(|shipping| !shipping.same_as_billing)
        .and_then(|shipping| shipping.address.as_ref())
        .map(|shipping| address(Some(shipping)));
    let emails = if customer.invoicing_emails.is_empty() {
        customer.billing_email.iter().cloned().collect()
    } else {
        customer.invoicing_emails.clone()
    };
    let external_reference = customer.id.as_proto();
    let billing_language = billing_language(&customer.preferred_locales);
    let phone = non_blank(customer.phone.as_deref());

    if customer.customer_type == CustomerType::Individual
        && let Some((first_name, last_name)) = person_names(customer)
    {
        return CustomerUpsert::Individual(IndividualCustomer {
            first_name,
            last_name,
            phone,
            billing_address,
            delivery_address,
            emails,
            external_reference,
            billing_language,
        });
    }

    CustomerUpsert::Company(CompanyCustomer {
        name: customer.name.clone(),
        vat_number: non_blank(customer.vat_number.as_deref()),
        reg_no: non_blank(customer.legal_number.as_deref()),
        phone,
        billing_address,
        delivery_address,
        emails,
        external_reference,
        billing_language,
    })
}

/// The person's names, or their display name split on its last word.
fn person_names(customer: &Customer) -> Option<(String, String)> {
    let first = non_blank(customer.first_name.as_deref());
    let last = non_blank(customer.last_name.as_deref());
    match (first, last) {
        (Some(first), Some(last)) => Some((first, last)),
        _ => {
            let name = customer.name.trim();
            name.rsplit_once(char::is_whitespace)
                .map(|(first, last)| (first.trim().to_owned(), last.to_owned()))
        }
    }
}

fn address(address: Option<&Address>) -> PennylaneAddress {
    let street = address
        .map(|a| {
            [a.line1.as_deref(), a.line2.as_deref()]
                .into_iter()
                .filter_map(non_blank)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    PennylaneAddress {
        address: street,
        postal_code: address
            .and_then(|a| non_blank(a.zip_code.as_deref()))
            .unwrap_or_default(),
        city: address
            .and_then(|a| non_blank(a.city.as_deref()))
            .unwrap_or_default(),
        country_alpha2: address
            .and_then(|a| a.country.as_ref())
            .map(|country| country.code.clone())
            .unwrap_or_default(),
    }
}

fn billing_language(locales: &[String]) -> Option<BillingLanguage> {
    locales.iter().find_map(|locale| {
        match locale.split(['-', '_']).next()?.to_lowercase().as_str() {
            "fr" => Some(BillingLanguage::French),
            "en" => Some(BillingLanguage::English),
            "de" => Some(BillingLanguage::German),
            "es" => Some(BillingLanguage::Spanish),
            _ => None,
        }
    })
}

fn non_blank(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
}

/// Where the document's VAT was levied: the destination for distance sales (OSS), else the
/// seller's own country.
pub(super) struct VatCountries<'a> {
    pub customer: Option<&'a str>,
    pub seller: &'a str,
}

/// The figures of an invoice or credit note, signed as stored (a credit note is negative).
pub(super) struct Document<'a> {
    pub number: &'a str,
    pub currency: &'a str,
    pub total: i64,
    pub tax_amount: i64,
    pub lines: &'a [LineItem],
    pub breakdown: &'a [TaxBreakdownItem],
    pub countries: VatCountries<'a>,
    /// Euros per unit of `currency` on the document's date, as its PDF states them.
    pub eur_rate: Option<Decimal>,
}

/// An import checked against Pennylane's rules, waiting for its PDF: the file is only uploaded once
/// the document is known to be importable.
#[derive(Debug)]
pub(super) struct PendingImport(CustomerInvoiceImport);

impl PendingImport {
    pub fn with_attachment(mut self, file_attachment_id: i64) -> CustomerInvoiceImport {
        self.0.file_attachment_id = file_attachment_id;
        self.0
    }
}

pub(super) fn invoice_import(
    invoice: &Invoice,
    customer_id: i64,
    transaction_reference: Option<TransactionReference>,
    eur_rate: Option<Decimal>,
) -> Result<PendingImport, Unsyncable> {
    let seller_country = seller_country(invoice.seller_details.address.country.as_ref());
    let document = Document {
        number: &invoice.invoice_number,
        currency: &invoice.currency,
        total: invoice.total,
        tax_amount: invoice.tax_amount,
        lines: &invoice.line_items,
        breakdown: &invoice.tax_breakdown,
        countries: VatCountries {
            customer: customer_country(invoice.customer_details.billing_address.as_ref()),
            seller: &seller_country,
        },
        eur_rate,
    };
    let deadline = invoice
        .due_at
        .map(|due| due.date())
        .filter(|due| *due >= invoice.invoice_date)
        .unwrap_or(invoice.invoice_date);

    import(
        &document,
        customer_id,
        Header {
            external_reference: invoice.id.as_proto(),
            date: invoice.invoice_date,
            deadline,
            credited_invoice_id: None,
            transaction_reference,
            purchase_order_reference: invoice
                .purchase_order
                .as_deref()
                .and_then(|po| non_blank(Some(po)))
                .map(|po| truncate(&po, MAX_REFERENCE_LEN)),
        },
    )
}

pub(super) fn credit_note_import(
    credit_note: &CreditNote,
    customer_id: i64,
    credited_invoice_id: i64,
    eur_rate: Option<Decimal>,
) -> Result<PendingImport, Unsyncable> {
    let seller_country = seller_country(credit_note.seller_details.address.country.as_ref());
    let document = Document {
        number: &credit_note.credit_note_number,
        currency: &credit_note.currency,
        total: credit_note.total,
        tax_amount: credit_note.tax_amount,
        lines: &credit_note.line_items,
        breakdown: &credit_note.tax_breakdown,
        countries: VatCountries {
            customer: customer_country(credit_note.customer_details.billing_address.as_ref()),
            seller: &seller_country,
        },
        eur_rate,
    };
    let date = credit_note
        .finalized_at
        .unwrap_or(credit_note.created_at)
        .date();

    import(
        &document,
        customer_id,
        Header {
            external_reference: credit_note.id.as_proto(),
            date,
            deadline: date,
            credited_invoice_id: Some(credited_invoice_id),
            transaction_reference: None,
            purchase_order_reference: None,
        },
    )
}

fn customer_country(address: Option<&Address>) -> Option<&str> {
    address
        .and_then(|a| a.country.as_ref())
        .map(|c| c.code.as_str())
}

fn seller_country(country: Option<&common_domain::country::CountryCode>) -> String {
    country.map(|c| c.code.clone()).unwrap_or_default()
}

struct Header {
    external_reference: String,
    date: NaiveDate,
    deadline: NaiveDate,
    credited_invoice_id: Option<i64>,
    transaction_reference: Option<TransactionReference>,
    purchase_order_reference: Option<String>,
}

fn import(
    document: &Document<'_>,
    customer_id: i64,
    header: Header,
) -> Result<PendingImport, Unsyncable> {
    let exponent = meteroid_money::iso::find(document.currency)
        .map(|currency| currency.exponent)
        .ok_or_else(|| Unsyncable(format!("Unknown currency {}", document.currency)))?;
    let money = |minor: i64| Amount(Decimal::new(minor, exponent));

    if document.lines.is_empty() {
        return Err(Unsyncable(format!("{} has no lines", document.number)));
    }
    if let Some(line) = document.lines.iter().find(|l| l.states_prepaid_credit()) {
        return Err(Unsyncable(format!(
            "{} sells or credits prepaid credit (\"{}\"), a customer advance rather than a sale: \
             record it in Pennylane by hand",
            document.number, line.name
        )));
    }

    let lines_total: i64 = document.lines.iter().map(|l| l.amount_total).sum();
    let lines_tax: i64 = document.lines.iter().map(|l| l.tax_amount).sum();
    if lines_total != document.total || lines_tax != document.tax_amount {
        return Err(Unsyncable(format!(
            "The lines of {} add up to {} with {} of tax, but the document states {} with {} of tax",
            document.number,
            money(lines_total).0,
            money(lines_tax).0,
            money(document.total).0,
            money(document.tax_amount).0,
        )));
    }

    let euros = document
        .eur_rate
        .filter(|_| document.currency != "EUR")
        .map(|rate| Euros::of(document, exponent, rate));

    let invoice_lines = document
        .lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            let (quantity, unit_price) = quantity_and_unit_price(line, exponent);
            Ok(InvoiceLine {
                label: line.name.clone(),
                quantity,
                raw_currency_unit_price: unit_price.to_string(),
                unit: String::new(),
                vat_rate: vat_rate(line, document.breakdown, &document.countries)?,
                currency_amount: money(line.amount_total),
                currency_tax: money(line.tax_amount),
                amount: euros.as_ref().map(|e| e.lines[index].0),
                tax: euros.as_ref().map(|e| e.lines[index].1),
                imputation_dates: imputation_dates(line),
            })
        })
        .collect::<Result<Vec<_>, Unsyncable>>()?;

    Ok(PendingImport(CustomerInvoiceImport {
        file_attachment_id: 0,
        customer_id,
        external_reference: header.external_reference,
        invoice_number: document.number.to_owned(),
        date: header.date,
        deadline: header.deadline,
        currency: document.currency.to_owned(),
        currency_amount_before_tax: money(document.total - document.tax_amount),
        currency_amount: money(document.total),
        currency_tax: money(document.tax_amount),
        amount: euros.as_ref().map(|e| e.total),
        tax: euros.as_ref().map(|e| e.tax),
        credited_invoice_id: header.credited_invoice_id,
        transaction_reference: header.transaction_reference,
        purchase_order_reference: header.purchase_order_reference,
        invoice_lines,
    }))
}

/// A document's euro figures: its totals converted, and its lines converted then nudged so they
/// add up to them, the difference going to the largest line.
struct Euros {
    total: Amount,
    tax: Amount,
    /// Amount and tax of each line.
    lines: Vec<(Amount, Amount)>,
}

impl Euros {
    fn of(document: &Document<'_>, exponent: u32, rate: Decimal) -> Euros {
        let convert = |minor: i64| {
            (Decimal::new(minor, exponent) * rate)
                .round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero)
        };
        let total = convert(document.total);
        let tax = convert(document.tax_amount);
        let mut amounts: Vec<Decimal> = document
            .lines
            .iter()
            .map(|l| convert(l.amount_total))
            .collect();
        let mut taxes: Vec<Decimal> = document
            .lines
            .iter()
            .map(|l| convert(l.tax_amount))
            .collect();
        if let Some(largest) = (0..document.lines.len())
            .max_by_key(|&index| document.lines[index].amount_total.unsigned_abs())
        {
            let (amounts_sum, taxes_sum): (Decimal, Decimal) =
                (amounts.iter().sum(), taxes.iter().sum());
            amounts[largest] += total - amounts_sum;
            taxes[largest] += tax - taxes_sum;
        }
        Euros {
            total: Amount(total),
            tax: Amount(tax),
            lines: amounts
                .into_iter()
                .zip(taxes)
                .map(|(amount, tax)| (Amount(amount), Amount(tax)))
                .collect(),
        }
    }
}

/// A line's period ends on the first day of the next one; Pennylane's end date is inclusive.
fn imputation_dates(line: &LineItem) -> Option<ImputationDates> {
    let end_date = if line.end_date > line.start_date {
        line.end_date.pred_opt()?
    } else {
        line.end_date
    };
    (line.start_date <= end_date).then_some(ImputationDates {
        start_date: line.start_date,
        end_date,
    })
}

/// The line's own quantity and unit price when they make its net amount, otherwise one unit at
/// that amount: a discount, credit or tax-inclusive netting moves the net away from the price.
fn quantity_and_unit_price(line: &LineItem, exponent: u32) -> (Decimal, Decimal) {
    let net = Decimal::new(line.amount_total - line.tax_amount, exponent);
    if let (Some(quantity), Some(unit_price)) = (line.quantity, line.unit_price)
        && !quantity.is_zero()
    {
        let unit_price = unit_price.round_dp(MAX_UNIT_PRICE_DECIMALS).normalize();
        for signed in [unit_price, -unit_price] {
            let amount = (quantity * signed)
                .round_dp_with_strategy(exponent, RoundingStrategy::MidpointAwayFromZero);
            if amount == net {
                return (quantity.normalize(), signed);
            }
        }
    }
    (Decimal::ONE, net)
}

/// The rate's treatment is recorded on the breakdown, a line only carries the rate: two supplies at
/// one rate are in the same position by construction, unless the breakdown says otherwise.
fn vat_rate(
    line: &LineItem,
    breakdown: &[TaxBreakdownItem],
    countries: &VatCountries<'_>,
) -> Result<VatRate, Unsyncable> {
    if line.tax_details.len() > 1 {
        let names: Vec<&str> = line
            .tax_details
            .iter()
            .map(|t| t.tax_name.as_str())
            .collect();
        return Err(Unsyncable(format!(
            "\"{}\" is taxed by several taxes ({}), which Pennylane cannot record on one line",
            line.name,
            names.join(", ")
        )));
    }

    let rows: Vec<&TaxBreakdownItem> = breakdown
        .iter()
        .filter(|row| row.tax_rate == line.tax_rate && !row.prepaid_credit)
        .collect();
    let mut exemptions: Vec<Option<&TaxExemptionType>> = vec![];
    for exemption in rows.iter().map(|row| row.exemption_type.as_ref()) {
        if !exemptions.contains(&exemption) {
            exemptions.push(exemption);
        }
    }
    let exemption = match exemptions.as_slice() {
        [] => None,
        [only] => *only,
        _ => {
            return Err(Unsyncable(format!(
                "The supplies at {}% are in different tax positions on this document, which \
                 Pennylane cannot tell apart (line \"{}\")",
                percent(line.tax_rate),
                line.name
            )));
        }
    };

    match exemption {
        Some(TaxExemptionType::ReverseCharge) => Ok(VatRate::CROSSBORDER),
        Some(TaxExemptionType::Export | TaxExemptionType::NoVatTerritory) => Ok(VatRate::EXTRACOM),
        Some(TaxExemptionType::TaxExempt) => Ok(VatRate::EXEMPT),
        Some(TaxExemptionType::NotRegistered) => Err(Unsyncable(format!(
            "No tax was applied to \"{}\" and the tax position was not determined (seller \
             country missing, or tax resolved outside Meteroid): Pennylane needs a VAT treatment",
            line.name
        ))),
        None if line.tax_rate.is_zero() => Ok(VatRate::EXEMPT),
        None => {
            // Only the engine's own VAT can be the destination's (OSS); a merchant's rate is theirs.
            let destination = rows
                .iter()
                .all(|row| row.tax_kind == Some(TaxKind::Vat) && row.tax_reference.is_none())
                && !rows.is_empty();
            let destination_code = countries
                .customer
                .filter(|_| destination)
                .and_then(|customer| VatRate::standard(customer, line.tax_rate));
            destination_code
                .or_else(|| VatRate::standard(countries.seller, line.tax_rate))
                .ok_or_else(|| {
                    Unsyncable(format!(
                        "Pennylane has no VAT code for {}% in {} (line \"{}\")",
                        percent(line.tax_rate),
                        countries.seller,
                        line.name
                    ))
                })
        }
    }
}

fn percent(rate: Decimal) -> Decimal {
    (rate * Decimal::ONE_HUNDRED).normalize()
}

fn truncate(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;
    use serde_json::json;

    fn line(json: serde_json::Value) -> LineItem {
        let mut base = json!({
            "local_id": "l1",
            "name": "Pro plan",
            "amount_subtotal": 10000,
            "taxable_amount": 10000,
            "tax_amount": 2000,
            "amount_total": 12000,
            "tax_rate": "0.2",
            "quantity": "2",
            "unit_price": "50",
            "start_date": "2026-10-01",
            "end_date": "2026-10-31",
            "sub_lines": [],
            "is_prorated": false,
        });
        base.as_object_mut()
            .unwrap()
            .extend(json.as_object().unwrap().clone());
        serde_json::from_value(base).unwrap()
    }

    fn row(rate: &str, exemption: Option<&str>) -> TaxBreakdownItem {
        serde_json::from_value(json!({
            "taxable_amount": 0,
            "tax_amount": 0,
            "tax_rate": rate,
            "name": "VAT",
            "exemption_type": exemption,
        }))
        .unwrap()
    }

    fn engine_vat(rate: &str) -> TaxBreakdownItem {
        TaxBreakdownItem {
            tax_kind: Some(TaxKind::Vat),
            ..row(rate, None)
        }
    }

    const FRANCE: VatCountries<'static> = VatCountries {
        customer: Some("FR"),
        seller: "FR",
    };

    fn document<'a>(
        lines: &'a [LineItem],
        breakdown: &'a [TaxBreakdownItem],
        countries: VatCountries<'a>,
    ) -> Document<'a> {
        Document {
            number: "INV-1",
            currency: "EUR",
            total: lines.iter().map(|l| l.amount_total).sum(),
            tax_amount: lines.iter().map(|l| l.tax_amount).sum(),
            lines,
            breakdown,
            countries,
            eur_rate: None,
        }
    }

    fn header() -> Header {
        Header {
            external_reference: "inv_1".into(),
            date: NaiveDate::from_ymd_opt(2026, 11, 1).unwrap(),
            deadline: NaiveDate::from_ymd_opt(2026, 11, 30).unwrap(),
            credited_invoice_id: None,
            transaction_reference: None,
            purchase_order_reference: None,
        }
    }

    fn import(
        document: &Document<'_>,
        customer_id: i64,
        header: Header,
    ) -> Result<CustomerInvoiceImport, Unsyncable> {
        super::import(document, customer_id, header).map(|pending| pending.with_attachment(2))
    }

    fn target() -> i64 {
        1
    }

    #[test]
    fn a_domestic_invoice_balances() {
        let lines = [
            line(json!({})),
            line(json!({
                "name": "API calls",
                "amount_subtotal": 1234,
                "taxable_amount": 1234,
                "tax_amount": 247,
                "amount_total": 1481,
                "quantity": null,
                "unit_price": null,
            })),
        ];
        let breakdown = [row("0.2", None)];
        let import = import(&document(&lines, &breakdown, FRANCE), target(), header()).unwrap();

        assert_eq!(import.currency_amount, Amount(dec!(134.81)));
        assert_eq!(import.currency_tax, Amount(dec!(22.47)));
        assert_eq!(import.currency_amount_before_tax, Amount(dec!(112.34)));
        assert_eq!(import.invoice_lines[0].vat_rate.code(), "FR_200");
        assert_eq!(import.invoice_lines[0].quantity, dec!(2));
        assert_eq!(import.invoice_lines[0].raw_currency_unit_price, "50");
        assert_eq!(import.invoice_lines[1].quantity, Decimal::ONE);
        assert_eq!(import.invoice_lines[1].raw_currency_unit_price, "12.34");
        assert_eq!(import.invoice_lines[1].currency_amount, Amount(dec!(14.81)));
    }

    #[test]
    fn a_discounted_line_is_one_unit_at_its_net() {
        let lines = [line(json!({
            "taxable_amount": 8000,
            "tax_amount": 1600,
            "amount_total": 9600,
        }))];
        let import = import(
            &document(&lines, &[row("0.2", None)], FRANCE),
            target(),
            header(),
        )
        .unwrap();
        assert_eq!(import.invoice_lines[0].quantity, Decimal::ONE);
        assert_eq!(import.invoice_lines[0].raw_currency_unit_price, "80.00");
    }

    #[test]
    fn a_credit_note_line_carries_the_sign_on_its_unit_price() {
        let lines = [line(json!({
            "amount_subtotal": -10000,
            "taxable_amount": -10000,
            "tax_amount": -2000,
            "amount_total": -12000,
        }))];
        let import = import(
            &document(&lines, &[row("0.2", None)], FRANCE),
            target(),
            header(),
        )
        .unwrap();
        assert_eq!(import.invoice_lines[0].quantity, dec!(2));
        assert_eq!(import.invoice_lines[0].raw_currency_unit_price, "-50");
        assert_eq!(import.currency_amount, Amount(dec!(-120.00)));
        assert_eq!(import.currency_amount_before_tax, Amount(dec!(-100.00)));
    }

    #[test]
    fn a_unit_price_is_cut_to_six_decimals() {
        let lines = [line(json!({
            "amount_subtotal": 1000,
            "taxable_amount": 1000,
            "tax_amount": 200,
            "amount_total": 1200,
            "quantity": "3",
            "unit_price": "3.333333333",
        }))];
        let import = import(
            &document(&lines, &[row("0.2", None)], FRANCE),
            target(),
            header(),
        )
        .unwrap();
        assert_eq!(import.invoice_lines[0].raw_currency_unit_price, "3.333333");
        assert_eq!(import.invoice_lines[0].quantity, dec!(3));
    }

    #[test]
    fn lines_that_do_not_add_up_are_refused() {
        let lines = [line(json!({}))];
        let mut document = document(&lines, &[], FRANCE);
        document.total = 12001;
        let err = import(&document, target(), header()).unwrap_err();
        assert!(err.0.contains("add up to 120.00"), "{}", err.0);
    }

    #[test]
    fn a_distance_sale_uses_the_destination_rate() {
        let lines = [line(
            json!({ "tax_rate": "0.19", "tax_amount": 1900, "amount_total": 11900 }),
        )];
        let countries = VatCountries {
            customer: Some("DE"),
            seller: "FR",
        };
        let import = import(
            &document(&lines, &[engine_vat("0.19")], countries),
            target(),
            header(),
        )
        .unwrap();
        assert_eq!(import.invoice_lines[0].vat_rate.code(), "DE_190");
    }

    #[test]
    fn a_rate_unknown_at_the_destination_falls_back_to_the_seller() {
        let lines = [line(json!({}))];
        let countries = VatCountries {
            customer: Some("US"),
            seller: "FR",
        };
        let import = import(
            &document(&lines, &[engine_vat("0.2")], countries),
            target(),
            header(),
        )
        .unwrap();
        assert_eq!(import.invoice_lines[0].vat_rate.code(), "FR_200");
    }

    #[test]
    fn a_rate_without_a_code_is_refused_with_its_countries() {
        let lines = [line(
            json!({ "tax_rate": "0.123", "tax_amount": 1230, "amount_total": 11230 }),
        )];
        let countries = VatCountries {
            customer: Some("DE"),
            seller: "FR",
        };
        let err = import(&document(&lines, &[], countries), target(), header()).unwrap_err();
        assert_eq!(
            err.0,
            "Pennylane has no VAT code for 12.3% in FR (line \"Pro plan\")"
        );
    }

    #[test]
    fn zero_tax_treatments_follow_the_breakdown() {
        let zero = json!({ "tax_rate": "0", "tax_amount": 0, "amount_total": 10000 });
        let cases = [
            (Some("ReverseCharge"), "crossborder"),
            (Some("Export"), "extracom"),
            (Some("NoVatTerritory"), "extracom"),
            (Some("TaxExempt"), "exempt"),
            (None, "exempt"),
        ];
        for (exemption, code) in cases {
            let lines = [line(zero.clone())];
            let import = import(
                &document(&lines, &[row("0", exemption)], FRANCE),
                target(),
                header(),
            )
            .unwrap();
            assert_eq!(
                import.invoice_lines[0].vat_rate.code(),
                code,
                "{exemption:?}"
            );
        }
    }

    #[test]
    fn a_merchant_rate_is_the_sellers_whatever_the_customers_country() {
        let lines = [line(json!({}))];
        let countries = VatCountries {
            customer: Some("AT"),
            seller: "FR",
        };
        let import = import(
            &document(&lines, &[row("0.2", None)], countries),
            target(),
            header(),
        )
        .unwrap();
        assert_eq!(import.invoice_lines[0].vat_rate.code(), "FR_200");
    }

    #[test]
    fn an_undetermined_tax_position_is_refused() {
        let lines = [line(
            json!({ "tax_rate": "0", "tax_amount": 0, "amount_total": 10000 }),
        )];
        let err = import(
            &document(&lines, &[row("0", Some("NotRegistered"))], FRANCE),
            target(),
            header(),
        )
        .unwrap_err();
        assert!(err.0.contains("not determined"), "{}", err.0);
    }

    #[test]
    fn two_positions_at_one_rate_are_refused() {
        let lines = [line(
            json!({ "tax_rate": "0", "tax_amount": 0, "amount_total": 10000 }),
        )];
        let breakdown = [row("0", Some("ReverseCharge")), row("0", Some("TaxExempt"))];
        let err = import(&document(&lines, &breakdown, FRANCE), target(), header()).unwrap_err();
        assert!(err.0.contains("different tax positions"), "{}", err.0);
    }

    #[test]
    fn prepaid_credit_is_refused() {
        let lines = [line(json!({
            "name": "Credit pack",
            "kind": "Credit",
            "tax_rate": "0",
            "tax_amount": 0,
            "amount_total": 10000,
        }))];
        let err = import(&document(&lines, &[], FRANCE), target(), header()).unwrap_err();
        assert!(
            err.0.contains("prepaid credit (\"Credit pack\")"),
            "{}",
            err.0
        );
    }

    #[test]
    fn a_period_ends_the_day_before_the_next_one_starts() {
        let lines = [line(json!({}))];
        let import = import(
            &document(&lines, &[row("0.2", None)], FRANCE),
            target(),
            header(),
        )
        .unwrap();
        let dates = import.invoice_lines[0].imputation_dates.as_ref().unwrap();
        assert_eq!(dates.start_date.to_string(), "2026-10-01");
        assert_eq!(dates.end_date.to_string(), "2026-10-30");
    }

    #[test]
    fn the_net_is_read_from_the_total_less_tax() {
        // A tax-inclusive 10.00 at 20%: 8.33 net, 1.67 tax; older lines lack a taxable amount.
        let lines = [line(json!({
            "amount_subtotal": 833,
            "taxable_amount": 0,
            "tax_amount": 167,
            "amount_total": 1000,
            "quantity": "1",
            "unit_price": "10",
        }))];
        let import = import(
            &document(&lines, &[row("0.2", None)], FRANCE),
            target(),
            header(),
        )
        .unwrap();
        assert_eq!(import.invoice_lines[0].quantity, Decimal::ONE);
        assert_eq!(import.invoice_lines[0].raw_currency_unit_price, "8.33");
    }

    #[test]
    fn a_currency_without_decimals_keeps_none() {
        let lines = [line(json!({
            "amount_subtotal": 1000,
            "taxable_amount": 1000,
            "tax_amount": 100,
            "amount_total": 1100,
            "tax_rate": "0.1",
            "quantity": "2",
            "unit_price": "500",
        }))];
        let breakdown = [row("0.1", None)];
        let mut document = document(&lines, &breakdown, FRANCE);
        document.currency = "JPY";
        let import = import(&document, target(), header()).unwrap();
        assert_eq!(import.currency_amount, Amount(dec!(1100)));
        assert_eq!(import.invoice_lines[0].raw_currency_unit_price, "500");
    }

    #[test]
    fn a_foreign_currency_document_carries_its_euro_figures() {
        let lines = [
            line(json!({
                "amount_subtotal": 3333,
                "taxable_amount": 3333,
                "tax_amount": 667,
                "amount_total": 4000,
                "quantity": null,
                "unit_price": null,
            })),
            line(json!({
                "amount_subtotal": 3333,
                "taxable_amount": 3333,
                "tax_amount": 667,
                "amount_total": 4000,
                "quantity": null,
                "unit_price": null,
            })),
            line(json!({
                "amount_subtotal": 3334,
                "taxable_amount": 3334,
                "tax_amount": 666,
                "amount_total": 4000,
                "quantity": null,
                "unit_price": null,
            })),
        ];
        let breakdown = [row("0.2", None)];
        let mut document = document(&lines, &breakdown, FRANCE);
        document.currency = "USD";
        document.eur_rate = Some(dec!(0.921234));
        let import = import(&document, target(), header()).unwrap();

        assert_eq!(import.amount, Some(Amount(dec!(110.55))));
        assert_eq!(import.tax, Some(Amount(dec!(18.42))));
        let line_amounts: Decimal = import
            .invoice_lines
            .iter()
            .map(|l| l.amount.unwrap().0)
            .sum();
        let line_taxes: Decimal = import.invoice_lines.iter().map(|l| l.tax.unwrap().0).sum();
        assert_eq!(line_amounts, dec!(110.55));
        assert_eq!(line_taxes, dec!(18.42));
    }

    #[test]
    fn a_euro_document_sends_no_conversion() {
        let lines = [line(json!({}))];
        let breakdown = [row("0.2", None)];
        let mut document = document(&lines, &breakdown, FRANCE);
        document.eur_rate = Some(dec!(1.1));
        let import = import(&document, target(), header()).unwrap();
        assert_eq!(import.amount, None);
        assert_eq!(import.invoice_lines[0].amount, None);
    }

    #[test]
    fn a_line_under_several_taxes_is_refused() {
        let lines = [line(json!({
            "tax_rate": "0.15",
            "tax_amount": 1500,
            "amount_total": 11500,
            "tax_details": [
                { "tax_rate": "0.05", "tax_name": "GST", "tax_amount": 500 },
                { "tax_rate": "0.10", "tax_name": "PST", "tax_amount": 1000 },
            ],
        }))];
        let err = import(&document(&lines, &[], FRANCE), target(), header()).unwrap_err();
        assert!(err.0.contains("GST, PST"), "{}", err.0);
    }

    #[test]
    fn a_document_without_lines_is_refused() {
        let err = import(&document(&[], &[], FRANCE), target(), header()).unwrap_err();
        assert_eq!(err.0, "INV-1 has no lines");
    }

    #[test]
    fn locales_pick_the_billing_language() {
        let locales = |values: &[&str]| values.iter().map(|v| v.to_string()).collect::<Vec<_>>();
        assert_eq!(
            billing_language(&locales(&["it-IT", "de-DE"])),
            Some(BillingLanguage::German)
        );
        assert_eq!(
            billing_language(&locales(&["fr"])),
            Some(BillingLanguage::French)
        );
        assert_eq!(billing_language(&locales(&["pt-BR"])), None);
    }

    #[test]
    fn an_address_joins_its_street_lines() {
        let address = address(Some(&Address {
            line1: Some("10 rue de la Paix".into()),
            line2: Some(" Bâtiment B ".into()),
            city: Some("Paris".into()),
            country: None,
            state: None,
            zip_code: Some("75002".into()),
        }));
        assert_eq!(address.address, "10 rue de la Paix, Bâtiment B");
        assert_eq!(address.country_alpha2, "");
    }
}
