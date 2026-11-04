//! What happened when a document was checked against an e-invoicing standard.
//!
//! # Why this is a report and not an error
//!
//! Billing is not conditional on paperwork. An invoice that cannot be expressed
//! as a Factur-X document is still a legally issued invoice: it is numbered, it
//! is owed, and it is sent. Refusing to issue it because a VAT number is
//! missing would take down billing over metadata.
//!
//! So generation never blocks issuance. What it produces instead is this: the
//! findings, in the standard's own vocabulary, attached to the document that
//! could not carry them. `BR-11 at BG-8/BT-55` names the field an operator has
//! to fill, which a boolean "not compliant" does not.
//!
//! # And why fixing it afterwards mostly does not work
//!
//! A finalized invoice is immutable — French law is explicit about it, and the
//! seller and customer identity are frozen onto the document at finalization.
//! Correcting the entity's registration number today does not make yesterday's
//! invoice compliant, because yesterday's invoice does not read from the entity
//! any more. Re-running generation on a failed document is therefore only
//! useful for a transient failure or a mapping fix, never for a missing field.
//!
//! The place a missing field *is* fixable is before finalization, while the
//! draft still tracks the live records. That is what
//! `meteroid_invoicing::einvoicing::check` is for, and why this type carries
//! the findings rather than swallowing them.

use crate::json_value_serde;
use chrono::NaiveDateTime;
use diesel_models::enums::EInvoicingStatusEnum;
use o2o::o2o;
use serde::{Deserialize, Serialize};

/// Whether the structured e-invoice was produced for a document.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, o2o)]
#[map_owned(EInvoicingStatusEnum)]
pub enum EInvoicingStatus {
    Generated,
    Failed,
}

/// One rule the document did not satisfy.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EInvoicingFinding {
    /// The authority's rule identifier — `BR-11`, `BR-CO-26`, `PEPPOL-EN16931-R003`.
    pub rule: String,
    /// Where, as a business term path: `BG-8/BT-55`, `BG-25[2]/BT-151`.
    pub term: String,
    /// The rule's own wording, verbatim, so it can be looked up in CEN's or
    /// KoSIT's index.
    pub message: String,
    /// Advice the rule text itself does not carry. Rare, and never filler.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

/// The outcome of checking a document against the entity's e-invoicing format.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EInvoicingReport {
    /// The standard the document was checked against, as the profile names it.
    pub profile: String,
    pub checked_at: NaiveDateTime,
    /// Empty on success. Every entry blocks the document from being issued in
    /// this format; warnings are not recorded, because they do not stop anything.
    #[serde(default)]
    pub findings: Vec<EInvoicingFinding>,
    /// Set when generation failed for a reason that is not a business rule —
    /// an unrepresentable amount, a PDF that could not be rewritten.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

json_value_serde!(EInvoicingReport);

impl EInvoicingReport {
    pub fn generated(profile: &str) -> Self {
        EInvoicingReport {
            profile: profile.to_string(),
            checked_at: chrono::Utc::now().naive_utc(),
            findings: Vec::new(),
            error: None,
        }
    }

    pub fn failed(profile: &str, findings: Vec<EInvoicingFinding>, error: Option<String>) -> Self {
        EInvoicingReport {
            profile: profile.to_string(),
            checked_at: chrono::Utc::now().naive_utc(),
            findings,
            error,
        }
    }
}

/// What a rendering run concluded about one document, for persistence.
#[derive(Debug, Clone)]
pub struct EInvoicingOutcome {
    pub status: EInvoicingStatus,
    pub report: EInvoicingReport,
}

impl EInvoicingOutcome {
    pub fn generated(profile: &str) -> Self {
        EInvoicingOutcome {
            status: EInvoicingStatus::Generated,
            report: EInvoicingReport::generated(profile),
        }
    }

    pub fn failed(profile: &str, findings: Vec<EInvoicingFinding>, error: Option<String>) -> Self {
        EInvoicingOutcome {
            status: EInvoicingStatus::Failed,
            report: EInvoicingReport::failed(profile, findings, error),
        }
    }
}
