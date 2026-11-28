//! What an invoice, a credit note and the documents a subscription change previews share
//! (ADR-0013 §11): their lines as the engine writes them — the sublines of tiers, packages and
//! dimension rates, the dimensions a usage line is grouped by — their taxes and coupons. Amounts
//! are integers in minor units, unit prices and quantities decimal strings. One rendering serves
//! issued documents and previewed ones.
use crate::AppState;
use crate::v2::Reader;
use crate::v2::customers::model::Address;
use crate::v2::problem::Problem;
use crate::v2::refs::{CouponRef, CreditUnitRef, ProductRef};
use axum::response::{IntoResponse, Response};
use common_domain::ids::{BaseId, ProductId, StoredDocumentId, TenantId};
use http::StatusCode;
use meteroid_store::domain::einvoicing::EInvoicingStatus as DomainEInvoicingStatus;
use meteroid_store::domain::invoice_lines::{
    ChargeType, LineItem, LineItemKind, SubLineAttributes, SubLineItem,
};
use meteroid_store::domain::{
    Address as DomainAddress, CouponLineItem, InlineCustomer, TaxBreakdownItem, TaxExemptionType,
};
use meteroid_store::repositories::products::ProductInterface;
use rust_decimal::Decimal;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use utoipa::ToSchema;

// ── Lines ───────────────────────────────────────────────────────────

/// What a line bills: `charge`, what a charge billed; `minimum_shortfall`, what brings the
/// period up to the subscription's minimum; `credit`, credit sold, which paying the document
/// funds.
#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LineKind {
    /// What a charge billed.
    Charge,
    /// What brings the period up to the subscription's minimum.
    MinimumShortfall,
    /// Credit sold: paying the document funds it.
    Credit,
}

/// How the charge a line bills recurs: `recurring`, a fee every period (a rate, seats);
/// `usage`, what the period's usage billed; `one_time`, billed once.
#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LineBilling {
    Recurring,
    Usage,
    OneTime,
}

/// How a tiered price reads its tiers: `graduated`, each unit at the price of the tier it falls
/// in; `volume`, every unit at the price of the tier the whole quantity falls in.
#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TierMode {
    /// Each unit at the price of the tier it falls in.
    Graduated,
    /// Every unit at the price of the tier the whole quantity falls in.
    Volume,
}

/// The tier a subline prices.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct SubLineTier {
    /// How the price reads its tiers.
    pub mode: TierMode,
    /// The unit the tier starts at.
    pub first_unit: u64,
    /// Absent for the last tier, which has no upper end.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_unit: Option<u64>,
    /// The tier's flat fee, billed once the quantity reaches it, in the currency's major unit.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub flat_fee: Option<Decimal>,
    /// The most the tier bills, its flat fee included, in the currency's major unit.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub flat_cap: Option<Decimal>,
}

/// A part of a line: a tier of a seat or usage price, a package, a rate of one dimension value.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct SubLine {
    /// Its id within its line: a credit note credits it by it (`lines[].sub_lines[].id`).
    pub id: String,
    /// As printed (`First 1000`, `Next 4000`, `Packs of 100`…).
    pub name: String,
    /// The units it prices, as a decimal string.
    #[schema(value_type = String, format = "decimal")]
    pub quantity: Decimal,
    /// In the currency's major unit.
    #[schema(value_type = String, format = "decimal")]
    pub unit_price: Decimal,
    /// In minor units, before discounts and tax.
    pub amount: i64,
    /// The tier it prices, on a tiered or volume price.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tier: Option<SubLineTier>,
    /// A package price's usage before it was rounded up to whole packages.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub raw_usage: Option<Decimal>,
    /// The dimension values a rate by dimension priced.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<BTreeMap<String, String>>,
}

/// Usage priced in a credit unit: the line's money amounts carry only what the customer's
/// credits did not cover, billed at `rate`.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct LineCredits {
    /// The credit unit the usage is priced in.
    pub credit_unit: CreditUnitRef,
    /// Credit amounts are integers in 10^-precision credit.
    pub precision: u8,
    /// Credits per usage unit.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub unit_price: Option<Decimal>,
    /// The currency's major units per credit.
    #[schema(value_type = String, format = "decimal")]
    pub rate: Decimal,
    /// The credit the line's usage consumed, in 10^-`precision` credit.
    pub consumed: i64,
    /// Of `consumed`, what the customer's credits paid.
    pub covered: i64,
}

/// A line of a document, as it was billed.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct DocumentLine {
    /// Its id within its document: a credit note names the invoice lines it credits by it, and a
    /// draft's edit the lines it keeps. A preview's lines have ids of their own, never issued.
    pub id: String,
    /// As printed: the charge's name, or the name a line written by hand was given.
    pub name: String,
    /// Printed under its name; absent when blank.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The product billed; absent on a line written by hand.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product: Option<ProductRef>,
    /// What the line bills.
    pub kind: LineKind,
    /// How the charge it bills recurs; absent on a line written by hand.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billing: Option<LineBilling>,
    /// Absent on a line its `sub_lines` add up (tiers, packages, dimension rates).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub quantity: Option<Decimal>,
    /// In the currency's major unit; absent as `quantity` is.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub unit_price: Option<Decimal>,
    /// In minor units, before discounts and tax.
    pub amount: i64,
    /// What is taxed of `amount`, discounts and promotional credit taken off.
    pub taxable_amount: i64,
    /// A fraction as a decimal string: `"0.2"` is 20%.
    #[schema(value_type = String, format = "decimal")]
    pub tax_rate: Decimal,
    /// The tax on `taxable_amount`, in minor units.
    pub tax: i64,
    /// `taxable_amount + tax`.
    pub total: i64,
    /// The first day the line bills.
    pub start_date: chrono::NaiveDate,
    /// The day after the last it bills: the period is `[start_date, end_date)`.
    pub end_date: chrono::NaiveDate,
    /// Billed for part of its period only (a change mid-period, a first period started late).
    pub prorated: bool,
    /// The values of the dimensions a usage line is grouped by.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<BTreeMap<String, String>>,
    /// Tiers, packages or dimension rates the line adds up.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sub_lines: Vec<SubLine>,
    /// The credit unit its usage is priced in, and what the customer's credits covered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credits: Option<LineCredits>,
}

/// The codes of the products lines bill.
#[derive(Debug, Clone, Default)]
pub struct ProductCodes(HashMap<ProductId, String>);

impl ProductCodes {
    pub async fn load<'a>(
        state: &impl Reader,
        tenant_id: TenantId,
        lines: impl IntoIterator<Item = &'a LineItem>,
    ) -> Result<Self, Problem> {
        let mut ids: Vec<ProductId> = lines.into_iter().filter_map(|l| l.product_id).collect();
        ids.sort_by_key(|id| id.as_uuid());
        ids.dedup();
        if ids.is_empty() {
            return Ok(Self::default());
        }
        Ok(Self(
            state
                .store()
                .find_products_by_ids(&ids, tenant_id)
                .await?
                .into_iter()
                .map(|product| (product.id, product.code))
                .collect(),
        ))
    }

    fn product(&self, id: ProductId) -> ProductRef {
        ProductRef {
            id,
            code: self.0.get(&id).cloned().unwrap_or_else(|| id.to_string()),
        }
    }
}

fn sub_line(sub_line: &SubLineItem) -> SubLine {
    let tier = |mode, first_unit, last_unit, flat_fee, flat_cap| SubLineTier {
        mode,
        first_unit,
        last_unit,
        flat_fee,
        flat_cap,
    };
    let (tier, raw_usage, dimensions) = match &sub_line.attributes {
        None => (None, None, None),
        Some(SubLineAttributes::Package { raw_usage }) => (None, Some(*raw_usage), None),
        Some(SubLineAttributes::Tiered {
            first_unit,
            last_unit,
            flat_cap,
            flat_fee,
        }) => (
            Some(tier(
                TierMode::Graduated,
                *first_unit,
                *last_unit,
                *flat_fee,
                *flat_cap,
            )),
            None,
            None,
        ),
        Some(SubLineAttributes::Volume {
            first_unit,
            last_unit,
            flat_cap,
            flat_fee,
        }) => (
            Some(tier(
                TierMode::Volume,
                *first_unit,
                *last_unit,
                *flat_fee,
                *flat_cap,
            )),
            None,
            None,
        ),
        Some(SubLineAttributes::Matrix {
            dimension1_key,
            dimension1_value,
            dimension2_key,
            dimension2_value,
        }) => {
            let mut dimensions =
                BTreeMap::from([(dimension1_key.clone(), dimension1_value.clone())]);
            if let (Some(key), Some(value)) = (dimension2_key, dimension2_value) {
                dimensions.insert(key.clone(), value.clone());
            }
            (None, None, Some(dimensions))
        }
    };
    SubLine {
        id: sub_line.local_id.clone(),
        name: sub_line.name.clone(),
        quantity: sub_line.quantity.normalize(),
        unit_price: sub_line.unit_price.normalize(),
        amount: sub_line.total,
        tier,
        raw_usage: raw_usage.map(|usage| usage.normalize()),
        dimensions,
    }
}

pub fn line(line: &LineItem, products: &ProductCodes) -> DocumentLine {
    DocumentLine {
        id: line.local_id.clone(),
        name: line.name.clone(),
        description: line.description.clone().filter(|d| !d.trim().is_empty()),
        product: line.product_id.map(|id| products.product(id)),
        kind: match line.kind {
            LineItemKind::Regular => LineKind::Charge,
            LineItemKind::MinimumShortfall => LineKind::MinimumShortfall,
            LineItemKind::Credit => LineKind::Credit,
        },
        billing: line.charge_type.map(|charge_type| match charge_type {
            ChargeType::Recurring => LineBilling::Recurring,
            ChargeType::Usage => LineBilling::Usage,
            ChargeType::OneTime => LineBilling::OneTime,
        }),
        quantity: line.quantity.map(|quantity| quantity.normalize()),
        unit_price: line.unit_price.map(|price| price.normalize()),
        amount: line.amount_subtotal,
        taxable_amount: line.taxable_amount,
        tax_rate: line.tax_rate.normalize(),
        tax: line.tax_amount,
        total: line.amount_total,
        start_date: line.start_date,
        end_date: line.end_date,
        prorated: line.is_prorated,
        dimensions: line
            .group_by_dimensions
            .as_ref()
            .filter(|dimensions| !dimensions.is_empty())
            .map(|dimensions| {
                dimensions
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect()
            }),
        sub_lines: line.sub_lines.iter().map(sub_line).collect(),
        credits: line.credit_unit.as_ref().map(|credit| LineCredits {
            credit_unit: CreditUnitRef {
                id: credit.unit_id,
                code: credit.code.clone(),
            },
            precision: credit.precision,
            unit_price: credit.unit_price.map(|price| price.normalize()),
            rate: credit.rate.normalize(),
            consumed: credit.consumed,
            covered: credit.covered,
        }),
    }
}

pub fn lines(lines: &[LineItem], products: &ProductCodes) -> Vec<DocumentLine> {
    lines.iter().map(|l| line(l, products)).collect()
}

// ── Taxes and coupons ───────────────────────────────────────────────

/// Why a tax applies at no rate: `reverse_charge`, the tax is due by the customer;
/// `tax_exempt`, the customer is exempt; `not_registered`, no tax applied and no reason is known
/// (no country, tax resolved outside Meteroid…); `export`, sold outside the seller's territory;
/// `no_vat_territory`, taxable where VAT does not apply in the seller's own territory.
#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaxExemption {
    ReverseCharge,
    TaxExempt,
    /// No tax applied, and no reason is known (no country, tax resolved outside Meteroid…).
    NotRegistered,
    Export,
    /// Taxable where VAT does not apply in the seller's own territory.
    NoVatTerritory,
}

/// One rate of the document's tax.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct DocumentTax {
    /// As printed (`VAT 20%`).
    pub name: String,
    /// A fraction as a decimal string: `"0.2"` is 20%.
    #[schema(value_type = String, format = "decimal")]
    pub rate: Decimal,
    /// What the rate applies to, in minor units: negative where credit or proration lines net
    /// below zero.
    pub taxable_amount: i64,
    /// The tax at this rate, in minor units.
    pub amount: i64,
    /// Why the rate is 0; absent on a tax applied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exemption: Option<TaxExemption>,
    /// The legal mention of the exemption.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exemption_reason: Option<String>,
    /// The rate's accounting code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
}

pub fn taxes(breakdown: &[TaxBreakdownItem]) -> Vec<DocumentTax> {
    breakdown
        .iter()
        .map(|item| DocumentTax {
            name: item.name.clone(),
            rate: item.tax_rate.normalize(),
            taxable_amount: item.taxable_amount,
            amount: item.tax_amount,
            exemption: item
                .exemption_type
                .as_ref()
                .map(|exemption| match exemption {
                    TaxExemptionType::ReverseCharge => TaxExemption::ReverseCharge,
                    TaxExemptionType::TaxExempt => TaxExemption::TaxExempt,
                    TaxExemptionType::NotRegistered => TaxExemption::NotRegistered,
                    TaxExemptionType::Export => TaxExemption::Export,
                    TaxExemptionType::NoVatTerritory => TaxExemption::NoVatTerritory,
                }),
            exemption_reason: item.exemption_reason.clone(),
            reference: item.tax_reference.clone(),
        })
        .collect()
}

/// A coupon the document applied.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct DocumentCoupon {
    /// The coupon applied; none for a discount an offer gives without a code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coupon: Option<CouponRef>,
    /// Its name, as printed.
    pub name: String,
    /// What it took off, in minor units, before tax.
    pub amount: i64,
}

pub fn coupons(coupons: &[CouponLineItem]) -> Vec<DocumentCoupon> {
    coupons
        .iter()
        .map(|coupon| DocumentCoupon {
            coupon: (!meteroid_store::domain::coupons::is_grant_code(
                coupon.coupon_id,
                &coupon.code,
            ))
            .then(|| CouponRef {
                id: coupon.coupon_id,
                code: coupon.code.clone(),
            }),
            name: coupon.name.clone(),
            amount: coupon.value,
        })
        .collect()
}

// ── Parties and files ───────────────────────────────────────────────

/// The customer a document is addressed to, as printed: as issued, or the live customer on a
/// draft. The customer itself is the document's `customer`.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct DocumentCustomer {
    /// The customer's id.
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub id: common_domain::ids::CustomerId,
    /// The integrator's own id of the customer; name it as `ext:<alias>` in requests.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    /// A company's name, or a person's full name.
    pub name: String,
    /// The billing email.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    /// The VAT number printed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vat_number: Option<String>,
    /// As the customer's own reads.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billing_address: Option<Address>,
}

fn address(address: &DomainAddress) -> Address {
    address.clone().into()
}

pub fn customer(details: &InlineCustomer) -> DocumentCustomer {
    DocumentCustomer {
        id: details.id,
        alias: details.alias.clone(),
        name: details.name.clone(),
        email: details.email.clone(),
        vat_number: details.vat_number.clone(),
        billing_address: details.billing_address.as_ref().map(address),
    }
}

/// The files of an issued document.
#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
pub struct DocumentFiles {
    /// The PDF is ready: `GET …/pdf`. Generated after the document is issued, so not at once.
    pub pdf: bool,
    /// The structured e-invoice (EN 16931 XML) is ready: `GET …/xml`.
    pub xml: bool,
}

/// Whether the structured e-invoice was produced with the PDF: `generated`, or `failed` when
/// the document did not satisfy the standard (the PDF was issued all the same).
#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EInvoicingStatus {
    Generated,
    /// Not produced: the document did not satisfy the standard (the PDF was issued all the same).
    Failed,
}

impl From<DomainEInvoicingStatus> for EInvoicingStatus {
    fn from(status: DomainEInvoicingStatus) -> Self {
        match status {
            DomainEInvoicingStatus::Generated => EInvoicingStatus::Generated,
            DomainEInvoicingStatus::Failed => EInvoicingStatus::Failed,
        }
    }
}

pub fn files(pdf: Option<StoredDocumentId>, xml: Option<StoredDocumentId>) -> DocumentFiles {
    DocumentFiles {
        pdf: pdf.is_some(),
        xml: xml.is_some(),
    }
}

async fn read_file(state: &AppState, id: StoredDocumentId) -> Result<bytes::Bytes, Problem> {
    meteroid_app::services::storage::read(state.object_store.as_ref(), &state.store, id)
        .await
        .map_err(|error| {
            log::error!("Failed to read a document's file: {error:?}");
            Problem::internal()
        })
}

/// A document's PDF, or `404` while it is not generated.
pub async fn pdf_response(
    state: &AppState,
    id: Option<StoredDocumentId>,
) -> Result<Response, Problem> {
    let id = id.ok_or_else(|| {
        Problem::not_found("No PDF yet: it is generated after the document is issued")
    })?;
    let data = read_file(state, id).await?;
    Ok((
        StatusCode::OK,
        [
            (http::header::CONTENT_TYPE, "application/pdf"),
            (http::header::CONTENT_DISPOSITION, "inline"),
        ],
        data,
    )
        .into_response())
}

/// A document's e-invoice XML, or `404` when it was not issued as one (or not yet).
pub async fn xml_response(
    state: &AppState,
    id: Option<StoredDocumentId>,
    number: &str,
) -> Result<Response, Problem> {
    let id = id.ok_or_else(|| {
        Problem::not_found(
            "No e-invoice XML: e-invoicing is off, the document was not issued as an e-invoice, \
             or generation is pending",
        )
    })?;
    let data = read_file(state, id).await?;
    Ok(crate::files::xml_download(data, number))
}

// ── Requests ────────────────────────────────────────────────────────

/// A moment a list is filtered by (RFC 3339), refused at its query parameter.
pub fn moment(raw: &str, parameter: &str) -> Result<chrono::DateTime<chrono::Utc>, Problem> {
    chrono::DateTime::parse_from_rfc3339(raw.trim())
        .map(|at| at.with_timezone(&chrono::Utc))
        .map_err(|_| {
            Problem::field(
                parameter,
                "invalid",
                "An RFC 3339 date-time, like 2026-12-01T00:00:00Z",
            )
        })
}

/// A reference a filter names, not found: refused at its query parameter.
pub fn filter_not_found(parameter: &'static str, raw: &str) -> impl FnOnce(Problem) -> Problem {
    let raw = raw.trim().to_owned();
    move |problem: Problem| {
        if problem.status == 404 {
            Problem::field(
                parameter,
                "not_found",
                format!("{parameter} {raw} not found"),
            )
        } else {
            problem
        }
    }
}

/// A document of `what` the path names, not found.
pub fn document_not_found(
    what: &'static str,
    id: impl std::fmt::Display,
) -> impl FnOnce(Problem) -> Problem {
    let id = id.to_string();
    move |problem: Problem| {
        if problem.status == 404 {
            Problem::not_found(format!("{what} {id}"))
        } else {
            problem
        }
    }
}
