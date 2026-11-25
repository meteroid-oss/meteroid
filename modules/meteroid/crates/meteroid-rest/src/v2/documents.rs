//! What an invoice, a credit note and the documents a subscription change previews share
//! (ADR-0013 §11): their lines as the engine writes them — the sublines of tiers, packages and
//! dimension rates, the dimensions a usage line is grouped by — their taxes and coupons. Amounts
//! are integers in minor units, unit prices and quantities decimal strings. One rendering serves
//! issued documents and previewed ones.
use crate::AppState;
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

/// How the charge a line bills recurs.
#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LineBilling {
    Recurring,
    Usage,
    OneTime,
}

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
    pub mode: TierMode,
    pub first_unit: u64,
    /// Unbounded when left out.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_unit: Option<u64>,
    /// In the currency's major unit.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub flat_fee: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub flat_cap: Option<Decimal>,
}

/// A part of a line: a tier of a seat or usage price, a package, a rate of one dimension value.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct SubLine {
    /// Its id within its line: a credit note credits it by it (`lines[].sub_lines[].id`).
    pub id: String,
    pub name: String,
    #[schema(value_type = String, format = "decimal")]
    pub quantity: Decimal,
    /// In the currency's major unit.
    #[schema(value_type = String, format = "decimal")]
    pub unit_price: Decimal,
    /// In minor units, before discounts and tax.
    pub amount: i64,
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
    pub consumed: i64,
    /// Of `consumed`, what the customer's credits paid.
    pub covered: i64,
}

/// A line of a document, as the engine wrote it.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct DocumentLine {
    /// Its id within its document: a credit note names the invoice lines it credits by it, and a
    /// draft's edit the lines it keeps. A preview's lines have ids of their own, never issued.
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The product billed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product: Option<ProductRef>,
    pub kind: LineKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billing: Option<LineBilling>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub quantity: Option<Decimal>,
    /// In the currency's major unit.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub unit_price: Option<Decimal>,
    /// In minor units, before discounts and tax.
    pub amount: i64,
    /// What is taxed of `amount`, discounts and promotional credit taken off.
    pub taxable_amount: i64,
    #[schema(value_type = String, format = "decimal")]
    pub tax_rate: Decimal,
    pub tax: i64,
    /// `taxable_amount + tax`.
    pub total: i64,
    pub start_date: chrono::NaiveDate,
    pub end_date: chrono::NaiveDate,
    pub prorated: bool,
    /// The values of the dimensions a usage line is grouped by.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<BTreeMap<String, String>>,
    /// Tiers, packages or dimension rates the line adds up.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sub_lines: Vec<SubLine>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credits: Option<LineCredits>,
}

/// The codes of the products lines bill.
#[derive(Debug, Clone, Default)]
pub struct ProductCodes(HashMap<ProductId, String>);

impl ProductCodes {
    pub async fn load<'a>(
        state: &AppState,
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
                .store
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

/// Why a tax applies at no rate.
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
    pub name: String,
    #[schema(value_type = String, format = "decimal")]
    pub rate: Decimal,
    pub taxable_amount: i64,
    pub amount: i64,
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
    pub coupon: CouponRef,
    /// Its name, as printed.
    pub name: String,
    /// What it took off, in minor units, before tax.
    pub amount: i64,
}

pub fn coupons(coupons: &[CouponLineItem]) -> Vec<DocumentCoupon> {
    coupons
        .iter()
        .map(|coupon| DocumentCoupon {
            coupon: CouponRef {
                id: coupon.coupon_id,
                code: coupon.code.clone(),
            },
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
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub id: common_domain::ids::CustomerId,
    /// The integrator's own id of the customer; name it as `ext:<alias>` in requests.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
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

/// Whether the structured e-invoice was produced with the PDF.
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
