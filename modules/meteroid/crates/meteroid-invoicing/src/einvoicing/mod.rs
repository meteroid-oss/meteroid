//! Structured e-invoicing: EN 16931 as the model, UBL / CII as the wire.
//!
//! Producing the document and transporting it are different jobs. We are not a
//! transport (a PDP, a Peppol access point, an agreed platform), and this module
//! is not one either — it turns a finalized invoice into the payload every
//! transport expects, and answers the question that comes before transport:
//! *is this invoice complete enough to be one?*
//!
//! That answer comes from [`en16931`], which carries the 317 business rules and
//! reports against them in the standard's own vocabulary — `BG-4/BT-31`, not
//! "seller VAT number missing". [`check`] is what drives the required-field
//! prompts; [`generate`] refuses to hand back a document that would fail them.
//!
//! The standard is not a country's invoicing law, and accepts documents a
//! country would not. [`national`] adds what the seller's own law requires, in
//! the same vocabulary and with the same effect.

mod codes;
mod facturx_xmp;
mod mapping;
mod national;

pub use facturx_xmp::stamp as stamp_facturx_xmp;

use crate::credit_note_model::CreditNote;
use crate::errors::InvoicingResult;
use crate::model::Invoice;
use en16931::validation::profile::Profile;
use en16931::{Identifier, ValidationReport};

pub use codes::VatCategory;

/// Which standard a tenant's invoices are issued under.
///
/// Not a scale. XRechnung and Peppol BIS are siblings — each a CIUS of
/// EN 16931 forbidding things the other allows — and Factur-X is the same core
/// model carried inside the PDF instead of beside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EInvoiceFormat {
    /// Factur-X / ZUGFeRD, EN 16931 (COMFORT) profile: CII embedded in the PDF.
    FacturX,
    /// The German CIUS, carried in CII so it can travel inside the PDF too.
    XRechnung,
    /// Peppol BIS Billing 3.0, carried in UBL. Transport-only — a Peppol
    /// document is not embedded in a PDF.
    PeppolBis,
}

/// The syntax a format is written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Syntax {
    Cii,
    Ubl,
}

impl EInvoiceFormat {
    fn profile(self) -> &'static Profile {
        match self {
            EInvoiceFormat::FacturX => &en16931::profiles::EN16931,
            EInvoiceFormat::XRechnung => &en16931::profiles::XRECHNUNG,
            EInvoiceFormat::PeppolBis => &en16931::profiles::PEPPOL_BIS_3,
        }
    }

    fn syntax(self) -> Syntax {
        match self {
            EInvoiceFormat::FacturX | EInvoiceFormat::XRechnung => Syntax::Cii,
            EInvoiceFormat::PeppolBis => Syntax::Ubl,
        }
    }

    /// The filename the payload must carry inside a hybrid PDF.
    ///
    /// The name is load-bearing, not descriptive: a receiver looks for these
    /// exact names in `/Names/EmbeddedFiles` and finds nothing under any other.
    pub fn attachment_filename(self) -> Option<&'static str> {
        match self {
            EInvoiceFormat::FacturX => Some("factur-x.xml"),
            EInvoiceFormat::XRechnung => Some("xrechnung.xml"),
            EInvoiceFormat::PeppolBis => None,
        }
    }

    /// Whether the payload belongs inside the PDF rather than beside it.
    pub fn is_hybrid(self) -> bool {
        self.attachment_filename().is_some()
    }

    /// The profile identifier as it appears in BT-24 and in the PDF's XMP.
    pub fn specification_id(self) -> &'static str {
        self.profile().specification_id
    }

    /// The profile's human name, for operator-facing messages.
    pub fn profile_name(self) -> &'static str {
        self.profile().id
    }

    /// The format a stored report belongs to. Reports record the profile name, not
    /// the format, and each format checks against its own profile — so the name is
    /// enough to recover it. `None` for a name no current format uses.
    pub fn from_profile_name(name: &str) -> Option<Self> {
        [Self::FacturX, Self::XRechnung, Self::PeppolBis]
            .into_iter()
            .find(|format| format.profile_name() == name)
    }

    /// `fx:ConformanceLevel` — how the profile is spelled in the PDF's XMP,
    /// which is not how BT-24 spells it. A receiver reads this to decide what
    /// it is about to parse.
    pub fn xmp_conformance_level(self) -> Option<&'static str> {
        match self {
            EInvoiceFormat::FacturX => Some("EN 16931"),
            EInvoiceFormat::XRechnung => Some("XRECHNUNG"),
            EInvoiceFormat::PeppolBis => None,
        }
    }
}

/// A generated e-invoice payload.
#[derive(Debug)]
pub struct EInvoiceDocument {
    pub xml: String,
    /// Filename for a hybrid PDF; `None` for transport-only formats.
    pub filename: Option<&'static str>,
    /// Terms the target syntax had no place for. Never silently discarded —
    /// non-empty here means the document is valid but says less than the model.
    pub dropped: Vec<String>,
}

impl EInvoiceDocument {
    pub fn mime_type(&self) -> &'static str {
        "application/xml"
    }
}

/// One rule a document did not satisfy, in the standard's own vocabulary.
///
/// Reported rather than raised: a document that cannot be expressed in a format
/// is still a legally issued document, and the caller's job is to record why,
/// not to refuse the invoice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// The authority's rule identifier — `BR-11`, `PEPPOL-EN16931-R003`.
    pub rule: String,
    /// The business term path it is about — `BG-8/BT-55`.
    pub term: String,
    /// The rule's own wording, verbatim.
    pub message: String,
    /// Advice the rule text does not carry. Rare.
    pub hint: Option<String>,
}

/// Why a document could not be issued in a format.
#[derive(Debug)]
pub enum EInvoiceRejection {
    /// The document is not valid under the profile. Every finding names a field.
    NotValid(Vec<Finding>),
    /// Something outside the business rules — an amount with too many decimals,
    /// a date the standard cannot express.
    Unmappable(String),
}

impl EInvoiceRejection {
    pub fn findings(&self) -> &[Finding] {
        match self {
            EInvoiceRejection::NotValid(findings) => findings,
            EInvoiceRejection::Unmappable(_) => &[],
        }
    }

    pub fn error(&self) -> Option<&str> {
        match self {
            EInvoiceRejection::NotValid(_) => None,
            EInvoiceRejection::Unmappable(message) => Some(message),
        }
    }
}

impl std::fmt::Display for EInvoiceRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EInvoiceRejection::NotValid(findings) => {
                let rules: Vec<String> = findings
                    .iter()
                    .map(|finding| format!("{} at {}", finding.rule, finding.term))
                    .collect();
                write!(f, "{}", rules.join(", "))
            }
            EInvoiceRejection::Unmappable(message) => f.write_str(message),
        }
    }
}

fn findings_of(report: &ValidationReport) -> Vec<Finding> {
    report
        .fatal()
        .map(|f| {
            let term = f.path.to_string();
            Finding {
                rule: f.rule.clone(),
                hint: f
                    .hint
                    .clone()
                    .or_else(|| where_to_fill(&term).map(str::to_owned)),
                term,
                message: f.message.clone(),
            }
        })
        .collect()
}

/// Where the operator fills a term we take from our own records. The rule text names
/// the term, and for `BR-DE-*` does so in German.
fn where_to_fill(term: &str) -> Option<&'static str> {
    match term.rsplit('/').next()? {
        "BT-10" => Some("Set the buyer reference (Leitweg-ID) on the customer."),
        "BT-41" | "BT-42" | "BT-43" | "BT-34" => {
            Some("Set the contact name, phone and email on the invoicing entity.")
        }
        "BT-49" => Some("Set a billing email on the customer."),
        "BG-16" => Some("Add a bank account to the invoicing entity."),
        _ => None,
    }
}

/// Validate an invoice against a format, without producing anything.
///
/// This is the required-field check, and the one worth running **before**
/// finalization: every finding names the business term it is about, so an
/// operator can fill the field while the draft is still editable. Afterwards it
/// is too late — a finalized document's identity is frozen onto it.
pub fn check(invoice: &Invoice, format: EInvoiceFormat) -> InvoicingResult<ValidationReport> {
    let mapped = addressed(
        mapping::to_en16931(invoice, format.specification_id())?,
        format,
    );
    Ok(format.profile().validate(&mapped))
}

/// XRechnung inherits Peppol's routing terms (BT-23, BT-34, BT-49) but, unlike Peppol,
/// accepts an email as an electronic address — so it needs no network registration.
/// Peppol gets nothing here: an `EM` address would validate and route nowhere.
fn addressed(mut mapped: en16931::Invoice, format: EInvoiceFormat) -> en16931::Invoice {
    if format == EInvoiceFormat::XRechnung {
        mapped.business_process = Some(codes::PEPPOL_BILLING_PROCESS.to_owned());
        for party in [&mut mapped.seller, &mut mapped.buyer] {
            party.electronic_address = party
                .contact
                .email
                .clone()
                .map(|email| Identifier::schemed(email, codes::EAS_EMAIL));
        }
    }
    mapped
}

/// The findings that would stop [`generate`], for an invoice that is still a draft:
/// the profile's rules, then the seller's national law (see [`national`]).
pub fn missing_requirements(
    invoice: &Invoice,
    format: EInvoiceFormat,
) -> InvoicingResult<Vec<Finding>> {
    let mut findings = findings_of(&check(invoice, format)?);
    findings.extend(national::findings(&invoice.organization, &invoice.customer));
    Ok(findings)
}

/// Adds the national findings to a write's outcome. A document the profile accepts
/// is still refused when national law is not met — the same gate as a profile rule,
/// so what a draft reported and what finalization records agree.
fn with_national(
    written: Result<EInvoiceDocument, EInvoiceRejection>,
    national: Vec<Finding>,
) -> Result<EInvoiceDocument, EInvoiceRejection> {
    match written {
        Ok(document) if national.is_empty() => Ok(document),
        Ok(_) => Err(EInvoiceRejection::NotValid(national)),
        Err(EInvoiceRejection::NotValid(mut findings)) => {
            findings.extend(national);
            Err(EInvoiceRejection::NotValid(findings))
        }
        Err(unmappable) => Err(unmappable),
    }
}

fn write(
    mapped: &en16931::Invoice,
    format: EInvoiceFormat,
) -> Result<EInvoiceDocument, EInvoiceRejection> {
    // The two syntaxes have their own `Written` type, so the arms are reduced to
    // what this crate carries rather than unified.
    let (xml, dropped) = match format.syntax() {
        Syntax::Cii => en16931_formats::cii::write_for(mapped, format.profile())
            .map(|w| (qualify_formatted_dates(w.xml), w.dropped)),
        Syntax::Ubl => {
            en16931_formats::ubl::write_for(mapped, format.profile()).map(|w| (w.xml, w.dropped))
        }
    }
    .map_err(|not_valid| EInvoiceRejection::NotValid(findings_of(not_valid.report())))?;

    Ok(EInvoiceDocument {
        xml,
        filename: format.attachment_filename(),
        dropped,
    })
}

/// CII types `ram:FormattedIssueDateTime` (BT-26) as a *qualified* date, and
/// `en16931-formats` (0.6, 0.7) writes it unqualified — an XSD failure on every credit
/// note and correction. Drop once upstream writes `qdt:`.
fn qualify_formatted_dates(xml: String) -> String {
    const OPEN: &str = "<ram:FormattedIssueDateTime>";
    const CLOSE: &str = "</ram:FormattedIssueDateTime>";
    let mut out = String::with_capacity(xml.len());
    let mut rest = xml.as_str();
    while let Some(start) = rest.find(OPEN) {
        let Some(len) = rest[start..].find(CLOSE) else {
            break;
        };
        let end = start + len;
        out.push_str(&rest[..start]);
        out.push_str(&rest[start..end].replace("udt:DateTimeString", "qdt:DateTimeString"));
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// Produce the e-invoice payload, or say which rules stopped it.
pub fn generate(
    invoice: &Invoice,
    format: EInvoiceFormat,
) -> Result<EInvoiceDocument, EInvoiceRejection> {
    let mapped = mapping::to_en16931(invoice, format.specification_id())
        .map(|mapped| addressed(mapped, format))
        .map_err(|e| EInvoiceRejection::Unmappable(e.to_string()))?;
    with_national(
        write(&mapped, format),
        national::findings(&invoice.organization, &invoice.customer),
    )
}

/// The credit-note twin of [`generate`]. BT-3 is `381` and BG-3 names the
/// invoice being credited.
pub fn generate_credit_note(
    credit_note: &CreditNote,
    format: EInvoiceFormat,
) -> Result<EInvoiceDocument, EInvoiceRejection> {
    let mapped = mapping::credit_note_to_en16931(credit_note, format.specification_id())
        .map(|mapped| addressed(mapped, format))
        .map_err(|e| EInvoiceRejection::Unmappable(e.to_string()))?;
    with_national(
        write(&mapped, format),
        national::findings(&credit_note.organization, &credit_note.customer),
    )
}

/// The CEN attribution notice. Carrying it is a condition of the licence the
/// EN 16931 artefacts are published under, so it is surfaced rather than kept
/// inside the dependency.
pub const ATTRIBUTION: &str = en16931_formats::ATTRIBUTION;
