//! National invoicing law, layered over EN 16931.
//!
//! EN 16931 is a semantic model, not any country's invoicing law, and it asks for
//! little: a buyer address may be a bare country code (BR-10/BR-11), and a seller
//! identified by VAT number alone needs no registration number (BR-CO-26). French
//! law asks more of every invoice, electronic or not, and a French document that
//! passes the standard can still be one the law would not accept.
//!
//! These rules report in the standard's own shape — a rule id and a business-term
//! path — and with the same effect as its rules: the invoice is not issued as an
//! e-invoice (the PDF still is), and on a draft the finding names the field to fill
//! while it can still be filled.
//!
//! Deliberately limited to requirements that hold for every invoice a French seller
//! issues. The mentions the 2026 e-invoicing reform adds (the customer's SIREN, the
//! category of operation, …) belong here once the transport that requires them does.

use super::Finding;
use crate::model::{Address, Customer, Organization};

/// Where a seller is registered under a SIREN: metropolitan France and the overseas
/// departments. Monaco is inside the French VAT territory but not the French company
/// register — its companies carry an RCI number — so it is deliberately absent.
const SIREN_TERRITORIES: &[&str] = &["FR", "GP", "MQ", "RE", "GF", "YT"];

/// Addresses with a French-style postcode. Elsewhere a postcode is not asked for:
/// some countries have none, and requiring one would fail invoices the law accepts.
const FRENCH_POSTCODE_TERRITORIES: &[&str] = &["FR", "GP", "MQ", "RE", "GF", "YT", "MC"];

const RULE_SIREN: &str = "FR-R123-237";
const RULE_ADDRESS: &str = "FR-242nA";

const ADDRESS_BASIS: &str = "CGI, annexe II, art. 242 nonies A: an invoice states the full \
                             name and address of the seller and of the customer";

/// The findings French law adds for this seller and buyer; empty for a seller
/// outside the French company register.
pub(crate) fn findings(seller: &Organization, buyer: &Customer) -> Vec<Finding> {
    if !in_territory(&seller.address, SIREN_TERRITORIES) {
        return Vec::new();
    }

    let mut findings = Vec::new();

    match seller
        .legal_number
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
    {
        None => findings.push(Finding {
            rule: RULE_SIREN.to_owned(),
            term: "BG-4/BT-30".to_owned(),
            message: "A French seller must state its SIREN on every invoice (Code de \
                      commerce, art. R123-237)."
                .to_owned(),
            hint: Some("Set the registration number on the invoicing entity.".to_owned()),
        }),
        Some(number) if !is_siren_or_siret(number) => findings.push(Finding {
            rule: RULE_SIREN.to_owned(),
            term: "BG-4/BT-30".to_owned(),
            message: format!(
                "The seller's registration number \"{number}\" is not a SIREN (9 digits) \
                 or a SIRET (14 digits) (Code de commerce, art. R123-237)."
            ),
            hint: Some("Correct the registration number on the invoicing entity.".to_owned()),
        }),
        Some(_) => {}
    }

    findings.extend(address_findings(
        &seller.address,
        Party::Seller,
        "Set it on the invoicing entity.",
    ));
    findings.extend(address_findings(
        &buyer.address,
        Party::Buyer,
        "Set it on the customer's billing address.",
    ));

    findings
}

#[derive(Clone, Copy)]
enum Party {
    Seller,
    Buyer,
}

impl Party {
    /// The party group, as the EN 16931 validator reports it (`BG-4/BT-40`,
    /// `BG-7/BT-55`), so both layers' findings read alike.
    fn group(self) -> &'static str {
        match self {
            Party::Seller => "BG-4",
            Party::Buyer => "BG-7",
        }
    }

    fn noun(self) -> &'static str {
        match self {
            Party::Seller => "seller's",
            Party::Buyer => "customer's",
        }
    }
}

/// Street, city and — for a French address — postcode. The country is left to the
/// standard, which already requires it (BR-09 for the seller, BR-11 for the buyer).
fn address_findings(address: &Address, party: Party, hint: &str) -> Vec<Finding> {
    let (line, city, postcode) = match party {
        Party::Seller => ("BT-35", "BT-37", "BT-38"),
        Party::Buyer => ("BT-50", "BT-52", "BT-53"),
    };
    let wants_postcode = in_territory(address, FRENCH_POSTCODE_TERRITORIES);

    [
        (line, "street address", blank(&address.line1), true),
        (city, "city", blank(&address.city), true),
        (
            postcode,
            "postcode",
            blank(&address.zip_code),
            wants_postcode,
        ),
    ]
    .into_iter()
    .filter(|(_, _, missing, required)| *missing && *required)
    .map(|(term, field, _, _)| Finding {
        rule: RULE_ADDRESS.to_owned(),
        term: format!("{}/{term}", party.group()),
        message: format!("The {} {field} is missing ({ADDRESS_BASIS}).", party.noun()),
        hint: Some(hint.to_owned()),
    })
    .collect()
}

fn in_territory(address: &Address, territories: &[&str]) -> bool {
    address
        .country
        .as_ref()
        .is_some_and(|c| territories.contains(&c.code.as_str()))
}

fn blank(value: &Option<String>) -> bool {
    value.as_deref().is_none_or(|v| v.trim().is_empty())
}

/// A SIREN is 9 digits and a SIRET 14 (the SIREN plus a 5-digit establishment
/// number), commonly written with spaces or dots. The Luhn key is deliberately not
/// checked: La Poste's SIREN famously fails it, and a false rejection here would
/// cost a valid invoice its e-invoice.
fn is_siren_or_siret(number: &str) -> bool {
    let digits: String = number
        .chars()
        .filter(|c| !matches!(c, ' ' | '.' | '-'))
        .collect();
    digits.chars().all(|c| c.is_ascii_digit()) && matches!(digits.len(), 9 | 14)
}

#[cfg(test)]
mod tests {
    use super::is_siren_or_siret;

    #[test]
    fn siren_and_siret_in_their_usual_spellings() {
        assert!(is_siren_or_siret("889123456"));
        assert!(is_siren_or_siret("889 123 456"));
        assert!(is_siren_or_siret("889 123 456 00015"));
        assert!(is_siren_or_siret("889.123.456.00015"));
        assert!(!is_siren_or_siret("RCS Paris 889 123 456"));
        assert!(!is_siren_or_siret("88912345"));
        assert!(!is_siren_or_siret("FR12889123456"));
    }
}
