//! Our invoice snapshot → the EN 16931 semantic model.
//!
//! The mapping is deliberately total and unvalidated: every term we hold goes
//! across, and whether the result is a legal invoice is [`en16931::validate`]'s
//! answer, not this module's. A mapper that refused to build an incomplete
//! model would report "missing field" in its own words, and the whole point of
//! the semantic model is that the finding says `BG-4/BT-31` instead.

use en16931::invoice::{
    Code, Contact, CreditTransfer, DocumentTotals, Item, LineAllowanceCharge, LineVat, Party,
    PaymentInstructions, PaymentMeans as En16931PaymentMeans, Period, PostalAddress,
    PrecedingInvoice, PriceDetails, VatBreakdown,
};
use en16931::{
    Date, DocumentReference, Identifier, InvoiceAmount, Percentage, Quantity, UnitPriceAmount,
};
use meteroid_money::iso::Currency;
use meteroid_money::{FormattableCurrency, Money};
use rust_decimal::Decimal;

use super::codes::{self, VatCategory};
use crate::credit_note_model::{CreditNote, CreditNoteLine, CreditType};
use crate::errors::{InvoicingError, InvoicingResult};
use crate::model::{
    Address, Customer, Invoice, InvoiceLine, Organization, TaxBreakdownItem, TaxExemptionType,
};

/// EN 16931 states every amount with at most two decimals (the `BR-DEC-*`
/// family), so a currency with a finer minor unit cannot be expressed at all.
/// Refusing is the honest answer; rounding would silently break the totals the
/// arithmetic rules check.
fn amount(money: &Money<'_, Currency>) -> InvoicingResult<InvoiceAmount> {
    InvoiceAmount::from_decimal_exact(*money.amount()).map_err(|e| {
        InvoicingError::XmlGenerationError(format!(
            "{} cannot be stated with two decimals, as EN 16931 requires: {e}",
            money.amount()
        ))
    })
}

fn date(d: chrono::NaiveDate) -> InvoicingResult<Date> {
    use chrono::Datelike;
    Date::new(d.year(), d.month() as u8, d.day() as u8)
        .map_err(|e| InvoicingError::XmlGenerationError(format!("unrepresentable date {d}: {e}")))
}

fn percentage(fraction: Decimal) -> InvoicingResult<Percentage> {
    Percentage::from_fraction(fraction).ok_or_else(|| {
        InvoicingError::XmlGenerationError(format!("tax rate {fraction} is not a valid percentage"))
    })
}

fn blank_to_none(s: Option<&String>) -> Option<String> {
    s.map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn postal_address(address: &Address) -> PostalAddress {
    PostalAddress {
        line1: blank_to_none(address.line1.as_ref()),
        line2: blank_to_none(address.line2.as_ref()),
        line3: None,
        city: blank_to_none(address.city.as_ref()),
        post_code: blank_to_none(address.zip_code.as_ref()),
        subdivision: blank_to_none(address.state.as_ref()),
        country: address.country.as_ref().map(|c| Code::new(c.code.as_str())),
    }
}

fn seller_party(org: &Organization, outside_scope: bool) -> Party {
    Party {
        name: Some(org.name.clone()),
        trading_name: None,
        identifiers: Vec::new(),
        legal_registration: blank_to_none(org.legal_number.as_ref()).map(Identifier::new),
        // BR-O-02: an out-of-scope document must not carry the seller's VAT
        // identifier (BT-31). The number still reaches the document, as the tax
        // registration identifier (BT-32), which BR-O leaves alone.
        vat_identifier: (!outside_scope)
            .then(|| blank_to_none(org.tax_id.as_ref()))
            .flatten(),
        tax_registration: outside_scope
            .then(|| blank_to_none(org.tax_id.as_ref()))
            .flatten(),
        additional_legal_information: blank_to_none(org.footer_legal.as_ref()),
        electronic_address: None,
        address: postal_address(&org.address),
        contact: Contact {
            name: blank_to_none(org.contact_name.as_ref()),
            phone: blank_to_none(org.contact_phone.as_ref()),
            email: blank_to_none(org.email.as_ref()),
        },
    }
}

fn buyer_party(customer: &Customer, outside_scope: bool) -> Party {
    Party {
        name: Some(customer.name.clone()),
        trading_name: None,
        identifiers: Vec::new(),
        legal_registration: blank_to_none(customer.legal_number.as_ref()).map(Identifier::new),
        // BR-O-04: nor the buyer's VAT identifier (BT-48).
        vat_identifier: (!outside_scope)
            .then(|| blank_to_none(customer.tax_id.as_ref()))
            .flatten(),
        tax_registration: None,
        additional_legal_information: None,
        electronic_address: None,
        address: postal_address(&customer.address),
        contact: Contact {
            name: None,
            phone: None,
            email: blank_to_none(customer.email.as_ref()),
        },
    }
}

/// The category a line belongs to, found by its rate in the invoice's own VAT
/// breakdown. The breakdown is where exemptions are recorded; a line only
/// carries a rate, and two supplies at the same rate are always in the same
/// category by construction. Credit sold has a row of its own, apart from the
/// supplies at its rate; a document predating that row finds it by rate.
fn line_category(
    rate: Decimal,
    prepaid_credit: bool,
    breakdown: &[TaxBreakdownItem],
) -> Result<VatCategory, codes::CategoryError> {
    breakdown
        .iter()
        .find(|item| item.rate == rate && item.prepaid_credit == prepaid_credit)
        .or_else(|| {
            breakdown
                .iter()
                .find(|item| prepaid_credit && item.rate == rate)
        })
        .map_or_else(
            || codes::category_of(None, rate),
            |item| codes::category_of(item.exemption_type.as_ref(), item.rate),
        )
}

/// BT-152 — stated as 0 for every zero-tax category except `O`, which forbids
/// it outright (BR-O-05).
fn line_vat(category: VatCategory, rate: Decimal) -> InvoicingResult<LineVat> {
    let rate = match category {
        VatCategory::OutsideScope => None,
        VatCategory::Standard => Some(percentage(rate)?),
        _ => Some(percentage(Decimal::ZERO)?),
    };
    Ok(LineVat {
        category: Code::new(category.code()),
        rate,
    })
}

/// The line terms EN 16931 has a place for, taken from either document model.
/// An invoice line and a credit-note line differ only in presentation fields the
/// standard has no term for — and in sign, which this normalises.
struct LineView<'a> {
    name: &'a str,
    description: Option<&'a String>,
    subtotal: Money<'a, Currency>,
    taxable_amount: Money<'a, Currency>,
    quantity: Option<Decimal>,
    unit_price: Option<Money<'a, Currency>>,
    tax_rate: Decimal,
    start_date: chrono::NaiveDate,
    end_date: chrono::NaiveDate,
    /// Credits a credit-priced line consumed and covered: the line only carries the overage.
    credit_note: Option<String>,
    /// Promotional credit within the line's reduction, positive.
    promotional_credit: Option<Money<'a, Currency>>,
    prepaid_credit: bool,
}

impl<'a> From<&'a InvoiceLine> for LineView<'a> {
    fn from(line: &'a InvoiceLine) -> Self {
        LineView {
            name: &line.name,
            description: line.description.as_ref(),
            subtotal: line.subtotal,
            taxable_amount: line.taxable_amount,
            // The money is a remainder of credits, not quantity × price: billed as one unit at
            // that remainder (PEPPOL-EN16931-R120), the usage quantity goes in the note.
            quantity: line.quantity.filter(|_| line.credit.is_none()),
            unit_price: line.unit_price,
            tax_rate: line.tax_rate,
            start_date: line.start_date,
            end_date: line.end_date,
            promotional_credit: line.promotional_credit,
            prepaid_credit: line.prepaid_credit,
            credit_note: line.credit.as_ref().map(|credit| {
                let usage = line
                    .quantity
                    .map(|quantity| format!("{} units, ", quantity.normalize()))
                    .unwrap_or_default();
                format!(
                    "{usage}{} consumed, {} covered by prepaid credits",
                    credit.quantity(credit.consumed),
                    credit.quantity(credit.covered),
                )
            }),
        }
    }
}

impl<'a> From<&'a CreditNoteLine> for LineView<'a> {
    /// Credit-note lines are stored negative — they subtract from an invoice —
    /// and are rendered that way. EN 16931 states a credit note **positive** and
    /// lets BT-3 = `381` carry the direction, so the sign is dropped here rather
    /// than travelling into a document that would fail `BR-27`.
    fn from(line: &'a CreditNoteLine) -> Self {
        LineView {
            name: &line.name,
            description: line.description.as_ref(),
            subtotal: line.subtotal.abs(),
            taxable_amount: line.taxable_amount.abs(),
            // Without a price the net price is the amount, so the line is one unit (R120).
            quantity: line.unit_price.and(line.quantity).map(|q| q.abs()),
            unit_price: line.unit_price.map(|p| p.abs()),
            tax_rate: line.tax_rate,
            start_date: line.start_date,
            end_date: line.end_date,
            credit_note: None,
            promotional_credit: line.promotional_credit.map(|credit| credit.abs()),
            prepaid_credit: false,
        }
    }
}

fn invoice_line(
    index: usize,
    line: &LineView<'_>,
    breakdown: &[TaxBreakdownItem],
) -> InvoicingResult<en16931::InvoiceLine> {
    let category = line_category(line.tax_rate, line.prepaid_credit, breakdown)
        .map_err(|e| InvoicingError::XmlGenerationError(format!("line {}: {e}", index + 1)))?;

    // A coupon or promotional credit allocated to this line is a line allowance (BG-27):
    // BT-131 is the amount after them, and each allowance says where its share went. Promotional
    // credit lowered the tax base, so it is an allowance (UNCL 5189 100), not a prepayment (BT-113).
    let reduction = *line.subtotal.amount() - *line.taxable_amount.amount();
    // Never above the reduction, which would leave a negative discount allowance.
    let promotional = line
        .promotional_credit
        .as_ref()
        .map_or(Decimal::ZERO, |credit| *credit.amount())
        .clamp(Decimal::ZERO, reduction.max(Decimal::ZERO));
    let mut allowances = Vec::new();
    for (value, reason, code) in [
        (reduction - promotional, "Discount", None),
        (
            promotional,
            "Promotional credit",
            Some(codes::ALLOWANCE_SPECIAL_REBATE),
        ),
    ] {
        if value.is_zero() {
            continue;
        }
        allowances.push(LineAllowanceCharge {
            amount: InvoiceAmount::from_decimal_exact(value).map_err(|e| {
                InvoicingError::XmlGenerationError(format!("line allowance {value}: {e}"))
            })?,
            base_amount: Some(amount(&line.subtotal)?),
            percentage: None,
            reason: Some(reason.to_owned()),
            reason_code: code.map(Code::new),
        });
    }

    Ok(en16931::InvoiceLine {
        id: (index + 1).to_string(),
        note: match (blank_to_none(line.description), &line.credit_note) {
            (note, None) => note,
            (None, Some(credit)) => Some(credit.clone()),
            (Some(note), Some(credit)) => Some(format!("{note} — {credit}")),
        },
        order_line_reference: None,
        accounting_reference: None,
        object_identifier: None,
        quantity: Quantity::new(line.quantity.unwrap_or(Decimal::ONE)),
        unit_code: Code::new(codes::UNIT_ONE),
        net_amount: amount(&line.taxable_amount)?,
        period: Some(Period {
            start: Some(date(line.start_date)?),
            end: Some(date(line.end_date)?),
        }),
        allowances,
        charges: Vec::new(),
        price: PriceDetails {
            net_price: line.unit_price.as_ref().map_or_else(
                || UnitPriceAmount::new(*line.subtotal.amount()),
                |p| UnitPriceAmount::new(*p.amount()),
            ),
            price_discount: None,
            gross_price: None,
            base_quantity: None,
            base_quantity_code: None,
        },
        vat: line_vat(category, line.tax_rate)?,
        item: Item {
            name: Some(line.name.to_owned()),
            description: None,
            seller_identifier: None,
            buyer_identifier: None,
            standard_identifier: None,
            classification_identifiers: Vec::new(),
            origin_country: None,
            attributes: Vec::new(),
        },
    })
}

/// The credit-note twin: the rendered breakdown carries negated tax amounts to
/// match the document, and EN 16931 wants them positive.
fn credit_note_vat_breakdown(
    item: &TaxBreakdownItem,
    legal: &crate::legal::LegalMentions,
    seller_in_eu: bool,
) -> InvoicingResult<VatBreakdown> {
    let mut out = vat_breakdown(item, legal, seller_in_eu)?;
    out.taxable_amount = out
        .taxable_amount
        .checked_abs()
        .map_err(|e| InvoicingError::XmlGenerationError(format!("taxable amount: {e}")))?;
    out.tax_amount = out
        .tax_amount
        .checked_abs()
        .map_err(|e| InvoicingError::XmlGenerationError(format!("tax amount: {e}")))?;
    Ok(out)
}

/// The statutory mention for a category, taken from the document's own resolved
/// wording. EN 16931 gives BT-120 one free-text field, so the headline and the basis
/// under it are joined: on the PDF they are two stacked lines saying one thing.
///
/// Falls back to `None` where the corpus has nothing for the category, letting the
/// caller drop to the merchant's own mention and then to the category default.
///
/// The category alone does not pick the basis: `O` covers both a supply taxable
/// outside the seller's territory and one taxable inside it where VAT does not apply
/// (CGI art. 294-1), and those cite different articles — so the exemption decides.
fn resolved_reason(
    legal: &crate::legal::LegalMentions,
    exemption: Option<&TaxExemptionType>,
    category: VatCategory,
) -> Option<String> {
    let joined = |headline: &str, basis: &str| {
        let basis = basis.trim();
        if basis.is_empty() {
            Some(headline.trim().to_owned())
        } else {
            Some(format!("{} — {}", headline.trim(), basis))
        }
    };
    match category {
        VatCategory::Standard | VatCategory::Zero => None,
        VatCategory::ReverseCharge => joined(
            &legal.reverse_charge_headline_resolved,
            &legal.reverse_charge_notice_resolved,
        ),
        VatCategory::Exempt => joined(&legal.vat_exempt_legal, &legal.vat_exempt_notice_resolved),
        VatCategory::OutsideScope
            if matches!(exemption, Some(TaxExemptionType::NoVatTerritory)) =>
        {
            joined(
                &legal.vat_not_applicable_legal,
                &legal.no_vat_territory_notice_resolved,
            )
        }
        VatCategory::OutsideScope => joined(
            &legal.vat_not_applicable_legal,
            &legal.export_notice_resolved,
        ),
    }
}

fn vat_breakdown(
    item: &TaxBreakdownItem,
    legal: &crate::legal::LegalMentions,
    seller_in_eu: bool,
) -> InvoicingResult<VatBreakdown> {
    let category = codes::category_of(item.exemption_type.as_ref(), item.rate)
        .map_err(|e| InvoicingError::XmlGenerationError(e.to_string()))?;
    // BT-119. Core EN 16931 permits omitting it for `O` (BR-O-05/06/07 govern the
    // *line* rate BT-152), but XRechnung's BR-DE-14 requires a rate on every BG-23
    // with no category exception, so the breakdown states 0 while the line omits it.
    let rate = match category {
        VatCategory::Standard => Some(percentage(item.rate)?),
        _ => Some(percentage(Decimal::ZERO)?),
    };

    // BT-120 or BT-121 is mandatory for every category that levies no tax. The
    // merchant's own mention wins; then the document's own statutory wording, so the
    // XML and the visual state the same position; then the bare category default.
    let (exemption_reason, exemption_reason_code) = if category.is_taxed() {
        (None, None)
    } else {
        (
            blank_to_none(item.exemption_reason.as_ref())
                .or_else(|| resolved_reason(legal, item.exemption_type.as_ref(), category))
                .or_else(|| category.default_reason().map(str::to_owned)),
            category.vatex_code(seller_in_eu).map(Code::new),
        )
    };

    Ok(VatBreakdown {
        taxable_amount: amount(&item.taxable_amount)?,
        tax_amount: amount(&item.amount)?,
        category: Code::new(category.code()),
        rate,
        exemption_reason,
        exemption_reason_code,
    })
}

fn payment_instructions(invoice: &Invoice) -> Option<PaymentInstructions> {
    let means = invoice.payment_means.as_ref()?;
    Some(PaymentInstructions {
        means_code: Some(Code::new(codes::PAYMENT_MEANS_CREDIT_TRANSFER)),
        means_text: None,
        remittance_information: blank_to_none(means.reference.as_ref()),
        means: Some(En16931PaymentMeans::CreditTransfer(vec![CreditTransfer {
            account_identifier: Some(means.account_identifier.clone()),
            account_name: blank_to_none(means.account_name.as_ref()),
            provider_identifier: blank_to_none(means.bic.as_ref()),
        }])),
    })
}

/// BG-16 on a credit note: how the credit is settled. XRechnung requires the group on
/// every document (BR-DE-1), and a credit-transfer code would demand the seller's
/// account (BG-17) for money that flows the other way.
fn credit_note_settlement(credit_type: &CreditType) -> PaymentInstructions {
    let code = match credit_type {
        CreditType::Refund => codes::PAYMENT_MEANS_MUTUALLY_DEFINED,
        CreditType::CreditToBalance | CreditType::DebtCancellation => codes::PAYMENT_MEANS_CLEARING,
    };
    PaymentInstructions {
        means_code: Some(Code::new(code)),
        means_text: None,
        remittance_information: None,
        means: None,
    }
}

/// BG-14 — the span the lines cover, which for a subscription invoice is the
/// billing period and is worth stating: it is what a receiver reconciles
/// against a contract.
fn invoicing_period(lines: &[LineView<'_>]) -> InvoicingResult<Option<Period>> {
    let start = lines.iter().map(|l| l.start_date).min();
    let end = lines.iter().map(|l| l.end_date).max();
    match (start, end) {
        (Some(start), Some(end)) => Ok(Some(Period {
            start: Some(date(start)?),
            end: Some(date(end)?),
        })),
        _ => Ok(None),
    }
}

/// BT-3 — a corrected invoice is `384`, not `380`: it replaces a document the
/// receiver already booked, and the code is how they know to.
fn type_code(invoice: &Invoice) -> &'static str {
    if invoice.metadata.parent_invoice_number.is_some() {
        codes::TYPE_CORRECTED_INVOICE
    } else {
        codes::TYPE_COMMERCIAL_INVOICE
    }
}

/// Whether the document is out of scope of the seller's VAT — the whole-document
/// property BR-O-11..14 enforce, which decides where the parties' numbers may go.
fn is_outside_scope(breakdown: &[TaxBreakdownItem]) -> bool {
    breakdown.iter().any(|item| {
        matches!(
            codes::category_of(item.exemption_type.as_ref(), item.rate),
            Ok(VatCategory::OutsideScope)
        )
    })
}

/// Whether the seller sits in the EU, which gates the VATEX catalogue (BT-121).
fn seller_in_eu(org: &Organization) -> bool {
    common_domain::country::TaxJurisdiction::of(
        org.address.country.as_ref().map(|c| c.code.as_str()),
    )
    .in_eu
}

/// One BG-23 per category and rate: the credit sold on a document has a row of its own in our
/// breakdown (kept out of the tax reports), and joins its category's group here, which states
/// the sum of every line in that category (BR-xx-08).
fn merged_vat_breakdown(
    breakdown: &[TaxBreakdownItem],
    legal: &crate::legal::LegalMentions,
    seller_in_eu: bool,
) -> InvoicingResult<Vec<VatBreakdown>> {
    let mut merged: Vec<((&'static str, Decimal), VatBreakdown)> = Vec::new();
    for item in breakdown {
        let category = codes::category_of(item.exemption_type.as_ref(), item.rate)
            .map_err(|e| InvoicingError::XmlGenerationError(e.to_string()))?;
        let rate = if category.is_taxed() {
            item.rate
        } else {
            Decimal::ZERO
        };
        let row = vat_breakdown(item, legal, seller_in_eu)?;
        match merged
            .iter_mut()
            .find(|(key, _)| *key == (category.code(), rate))
        {
            Some((_, group)) => {
                let sum = |a: &InvoiceAmount, b: &InvoiceAmount| {
                    InvoiceAmount::checked_sum(vec![*a, *b]).map_err(|e| {
                        InvoicingError::XmlGenerationError(format!("VAT breakdown overflowed: {e}"))
                    })
                };
                group.taxable_amount = sum(&group.taxable_amount, &row.taxable_amount)?;
                group.tax_amount = sum(&group.tax_amount, &row.tax_amount)?;
            }
            None => merged.push(((category.code(), rate), row)),
        }
    }
    Ok(merged.into_iter().map(|(_, row)| row).collect())
}

pub fn to_en16931(invoice: &Invoice, specification_id: &str) -> InvoicingResult<en16931::Invoice> {
    let meta = &invoice.metadata;
    let currency = meta.currency.code();
    let outside_scope = is_outside_scope(&invoice.tax_breakdown);
    let in_eu = seller_in_eu(&invoice.organization);

    let mut builder = en16931::Invoice::builder(
        specification_id,
        meta.number.clone(),
        date(meta.issue_date)?,
        Code::new(type_code(invoice)),
        Code::new(currency),
    )
    .due_date(date(meta.due_date)?)
    .seller(seller_party(&invoice.organization, outside_scope))
    .buyer(buyer_party(&invoice.customer, outside_scope));

    let lines: Vec<LineView<'_>> = invoice.lines.iter().map(LineView::from).collect();
    for (index, line) in lines.iter().enumerate() {
        builder = builder.line(invoice_line(index, line, &invoice.tax_breakdown)?);
    }
    // Resolved once: BT-120 must state the same tax position the PDF prints.
    let legal = crate::typst_render::resolve_legal_mentions(invoice);
    for row in merged_vat_breakdown(&invoice.tax_breakdown, &legal, in_eu)? {
        builder = builder.vat_breakdown(row);
    }

    if let Some(memo) = blank_to_none(meta.memo.as_ref()) {
        builder = builder.note(memo);
    }
    if let Some(reference) = blank_to_none(invoice.customer.buyer_reference.as_ref()) {
        builder = builder.buyer_reference(reference);
    }
    if let Some(po) = blank_to_none(meta.purchase_order.as_ref()) {
        builder = builder.purchase_order_reference(DocumentReference::new(po));
    }
    if let Some(period) = invoicing_period(&lines)? {
        builder = builder.invoicing_period(period);
    }
    if let Some(payment) = payment_instructions(invoice) {
        builder = builder.payment(payment);
    }
    if invoice.organization.accounting_currency.code() != currency {
        builder = builder
            .vat_accounting_currency(Code::new(invoice.organization.accounting_currency.code()));
    }

    // BT-106 is the sum of the line net amounts, which are already net of their
    // allowances — so there is no document-level allowance to state, and
    // BT-109 equals BT-106.
    let line_total = line_total(&lines)?;

    let prepaid = amount(&meta.prepaid_amount)?;
    let gross = amount(&meta.total_amount)?;
    let due = gross
        .checked_sub(prepaid)
        .map_err(|e| InvoicingError::XmlGenerationError(format!("amount due: {e}")))?;

    builder = builder.totals(DocumentTotals {
        line_total,
        allowance_total: None,
        charge_total: None,
        taxable_total: line_total,
        vat_total: Some(amount(&meta.tax_amount)?),
        vat_total_accounting: None,
        gross_total: gross,
        paid: if prepaid.is_zero() {
            None
        } else {
            Some(prepaid)
        },
        rounding: None,
        due,
    });

    let mut built = builder.build();

    // BG-3 — a corrected invoice must name the one it corrects (BR-55).
    if let Some(reference) = blank_to_none(meta.parent_invoice_number.as_ref()) {
        built.preceding_invoices = vec![PrecedingInvoice {
            reference: DocumentReference::new(reference),
            issue_date: meta.parent_invoice_date.map(date).transpose()?,
        }];
    }

    Ok(built)
}

fn line_total(lines: &[LineView<'_>]) -> InvoicingResult<InvoiceAmount> {
    InvoiceAmount::checked_sum(
        lines
            .iter()
            .map(|l| amount(&l.taxable_amount))
            .collect::<InvoicingResult<Vec<_>>>()?,
    )
    .map_err(|e| InvoicingError::XmlGenerationError(format!("line total overflowed: {e}")))
}

/// A credit note is the same model with a different type code and a mandatory
/// reference to what it credits — not a negative invoice. Its amounts are
/// stated **positive**; BT-3 = `381` is what tells the receiver to subtract.
pub fn credit_note_to_en16931(
    credit_note: &CreditNote,
    specification_id: &str,
) -> InvoicingResult<en16931::Invoice> {
    let meta = &credit_note.metadata;
    let currency = meta.currency.code();
    let outside_scope = is_outside_scope(&credit_note.tax_breakdown);
    let in_eu = seller_in_eu(&credit_note.organization);

    let mut builder = en16931::Invoice::builder(
        specification_id,
        meta.number.clone(),
        date(meta.issue_date)?,
        Code::new(codes::TYPE_CREDIT_NOTE),
        Code::new(currency),
    )
    .credit_note()
    .seller(seller_party(&credit_note.organization, outside_scope))
    .buyer(buyer_party(&credit_note.customer, outside_scope));

    let lines: Vec<LineView<'_>> = credit_note.lines.iter().map(LineView::from).collect();
    for (index, line) in lines.iter().enumerate() {
        builder = builder.line(invoice_line(index, line, &credit_note.tax_breakdown)?);
    }
    let legal = crate::credit_note_render::resolve_legal_mentions(credit_note);
    for item in &credit_note.tax_breakdown {
        builder = builder.vat_breakdown(credit_note_vat_breakdown(item, &legal, in_eu)?);
    }

    if let Some(reference) = blank_to_none(credit_note.customer.buyer_reference.as_ref()) {
        builder = builder.buyer_reference(reference);
    }
    for note in [meta.reason.as_ref(), meta.memo.as_ref()]
        .into_iter()
        .flatten()
    {
        if let Some(note) = blank_to_none(Some(note)) {
            builder = builder.note(note);
        }
    }
    if let Some(period) = invoicing_period(&lines)? {
        builder = builder.invoicing_period(period);
    }
    builder = builder.payment(credit_note_settlement(&meta.credit_type));
    if credit_note.organization.accounting_currency.code() != currency {
        builder = builder.vat_accounting_currency(Code::new(
            credit_note.organization.accounting_currency.code(),
        ));
    }

    // Nothing is owed on a credit note, so BT-115 is zero and BR-CO-25 — which
    // wants a due date or payment terms only for a positive balance — does not
    // apply. The credited amount is BT-112, where a receiver books it.
    let line_total = line_total(&lines)?;
    let gross = amount(&meta.total_amount.abs())?;

    builder = builder.totals(DocumentTotals {
        line_total,
        allowance_total: None,
        charge_total: None,
        taxable_total: line_total,
        vat_total: Some(amount(&meta.tax_amount.abs())?),
        vat_total_accounting: None,
        gross_total: gross,
        paid: Some(gross),
        rounding: None,
        due: InvoiceAmount::from_minor_units(0),
    });

    let mut built = builder.build();

    // BG-3 — a credit note that names no invoice is unbookable (BR-55).
    built.preceding_invoices = vec![PrecedingInvoice {
        reference: DocumentReference::new(meta.related_invoice_number.clone()),
        issue_date: Some(date(meta.related_invoice_date)?),
    }];

    Ok(built)
}
