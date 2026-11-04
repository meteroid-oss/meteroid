//! Jurisdiction-keyed legal corpus.
//!
//! Statutory invoice mentions (VAT exemption, export, reverse-charge, tax labels)
//! are *selected* by the seller's tax jurisdiction — its national regime, whether
//! it operates VAT, and EU membership — while their *wording* comes from the
//! presentation-language message bundle. This module owns only the selection: the
//! caller fills [`LegalCandidates`] from its bundle, and [`LegalMentions::resolve`]
//! picks the ones that apply.
//!
//! Keeping selection here (rather than inline in each renderer) keeps invoice and
//! credit-note output in lockstep and makes "onboard a country's mentions" a
//! single-site change. It is intentionally pure — no i18n or rendering types — so
//! the decision table is testable on its own.
//!
//! Every EU-specific candidate is reachable only through an `in_eu` branch here, so a
//! seller outside the EU cannot be handed directive wording by a template that forgot
//! to gate it. `non_eu_seller_never_sees_eu_wording` holds that line over every
//! country code.

use common_domain::country::{LegalRegime, TaxJurisdiction};

/// Whether the customer is a business, which decides the place-of-supply article on a
/// cross-border supply: a supply to a taxable person is located where the customer is
/// established, a supply to a private person generally is not.
///
/// Renderers take this from the basis the tax engine recorded on the breakdown, so the
/// article a document cites always matches the basis the tax was computed on. They must
/// not re-derive it: the engine classifies on the live customer record, while the
/// document renders an invoice snapshot an operator can edit without the engine
/// re-running, and the two then disagree — an invoice asserting the customer is a
/// "non-assujetti" for a supply priced as B2B, or citing the business place-of-supply
/// article over a consumer computation.
///
/// [`Self::from_tax_id`] remains for documents written before the basis was recorded.
/// It is a weaker signal by construction — tax-ID presence alone, with no format check —
/// and is deliberately not the engine's test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CustomerKind {
    Business,
    Consumer,
}

impl CustomerKind {
    /// Fallback for documents whose breakdown predates the recorded basis. Prefer the
    /// engine's own finding wherever the breakdown carries one.
    pub fn from_tax_id(customer_tax_id: Option<&str>) -> Self {
        match customer_tax_id.map(str::trim).filter(|id| !id.is_empty()) {
            Some(_) => CustomerKind::Business,
            None => CustomerKind::Consumer,
        }
    }
}

/// The statute citations a national regime puts on documents, in the presentation
/// language. A regime that has none reads as [`LegalRegime::Generic`] and falls back
/// to the EU-directive (or plain tax) wording.
///
/// Only the place-of-supply citation is national. The mention it sits under is a plain
/// statement ("VAT not applicable") and carries no article of its own: the article
/// depends on *why* no tax applied, which differs per branch, and repeating a
/// place-of-supply article above a different one below is how the two lines end up
/// contradicting each other.
#[derive(Default)]
pub struct NationalCitations {
    /// Cross-border supply to a business, carrying the national article. `None` where
    /// the regime contributes labels but no citation of its own (Austria).
    pub export_notice_b2b: Option<String>,
    /// Cross-border supply to a private customer. `None` where the regime has no
    /// settled citation for it, which falls through to the generic wording rather than
    /// reusing the business article — they are different provisions.
    pub export_notice_b2c: Option<String>,
    /// Registration-number labels, where the national term is not the EU "VAT
    /// identification number" the generic keys assume. Switzerland and Liechtenstein
    /// issue an MWST-Nr., Austria a UID-Nummer, Portugal a NIF; a shared language
    /// bundle cannot know which, because the label follows the seller's jurisdiction,
    /// not the language.
    ///
    /// Unlike the citations, a label is not a VAT concept: Brazil operates no VAT and
    /// still names its registration number (the CNPJ), so labels are the one thing a
    /// regime contributes whether or not the jurisdiction operates VAT.
    pub seller_tax_label: Option<String>,
    pub customer_tax_label: Option<String>,
    /// Breakdown-row label for engine-resolved VAT, where the national abbreviation is
    /// not the one the language bundle would pick. The shared `de` bundle says "USt.";
    /// Swiss usage (and the FTA's own documents) write "MWST" — and which one applies
    /// follows the seller's jurisdiction, not the document's language, exactly like the
    /// registration-number labels above. Unlike those labels this *is* a VAT concept,
    /// so it sits behind the `uses_vat` gate: a non-VAT regime has no VAT to name.
    pub vat_name: Option<String>,
    /// Headline above the citation, where the national statutory category is not the
    /// generic "VAT not applicable". Swiss law distinguishes *nicht steuerbar* (place
    /// abroad, art. 8), *ausgenommen* (art. 21) and *befreit* (art. 23); only the first
    /// describes what this branch renders.
    pub vat_not_applicable: Option<String>,
}

/// National citations for every regime the corpus knows. Callers fill all of them
/// from their bundle; [`LegalCandidates`] keeps only the seller's.
pub struct NationalCandidates {
    pub france: NationalCitations,
    pub belgium: NationalCitations,
    pub switzerland: NationalCitations,
    pub italy: NationalCitations,
    pub austria: NationalCitations,
    pub portugal: NationalCitations,
    pub brazil: NationalCitations,
}

impl NationalCandidates {
    fn select(self, regime: LegalRegime) -> Option<NationalCitations> {
        match regime {
            LegalRegime::Generic => None,
            LegalRegime::France => Some(self.france),
            LegalRegime::Belgium => Some(self.belgium),
            LegalRegime::Switzerland => Some(self.switzerland),
            LegalRegime::Italy => Some(self.italy),
            LegalRegime::Austria => Some(self.austria),
            LegalRegime::Portugal => Some(self.portugal),
            LegalRegime::Brazil => Some(self.brazil),
        }
    }
}

/// Every statutory-mention string available in the presentation language, before
/// jurisdiction selection. Callers fill this from their message bundle; the corpus
/// then keeps only what the seller's jurisdiction requires.
///
/// Credit-note renderers fill the document-type-sensitive fields
/// (`reverse_charge_notice*`) from their own credit-note keys, so one selection table
/// serves both document types.
pub struct LegalCandidates {
    /// Citations of each national regime, before the seller's is picked.
    pub national: NationalCandidates,
    /// Generic "VAT not applicable" — the headline over a *place-of-supply* mention.
    pub vat_not_applicable: String,
    /// Generic "tax not applicable" (non-VAT systems).
    pub tax_not_applicable: String,
    /// Headline over an *exemption* mention. A supply outside the scope and an exempt
    /// supply are different legal categories in every system the corpus covers — Spanish
    /// *no sujeción* vs *exención*, Swiss art. 8 *nicht steuerbar* vs art. 21
    /// *ausgenommen*, German *nicht steuerbar* vs *steuerfrei* — so they cannot share
    /// one headline without the document miscategorising one of them.
    pub vat_exempt_headline: String,
    /// Exemption headline for non-VAT systems.
    pub tax_exempt_headline: String,
    /// EU seller supplying a business outside the EU (Directive art. 44).
    pub export_notice_generic: String,
    /// EU seller supplying a private customer outside the EU (Directive art. 58).
    pub export_notice_generic_b2c: String,
    /// Cross-border supply from a VAT country outside the EU: no directive citation.
    pub export_notice_non_eu: String,
    /// Cross-border supply from a country with no VAT, worded as generic tax so the
    /// mention and the headline above it never speak two different tax vocabularies.
    pub export_notice_no_vat: String,
    /// Supply taxable in Guyane or Mayotte, where VAT does not apply (CGI art. 294-1).
    pub no_vat_territory_notice: String,
    /// VAT-worded exemption footnote.
    pub vat_exempt_notice: String,
    /// Tax-worded exemption footnote (non-VAT systems).
    pub tax_exempt_notice_generic: String,
    pub seller_vat_id: String,
    pub seller_tax_id: String,
    pub customer_vat_id: String,
    pub customer_tax_id: String,
    /// The presentation language's own word for VAT ("TVA", "USt.", "btw"): the
    /// breakdown-row label for a rate the engine resolved as VAT. A row label, not a
    /// mention — but resolved here so the Swiss "MWST" override can follow the
    /// seller's jurisdiction the way the registration-number labels do.
    pub vat_name: String,
    /// "prices include VAT" line.
    pub tax_included_text: String,
    /// "prices include tax" line (non-VAT systems).
    pub tax_included_generic: String,
    /// Reverse charge under the EU VAT Directive (arts 44 and 196).
    pub reverse_charge_notice: String,
    /// Reverse charge outside the EU, carrying no directive citation.
    pub reverse_charge_notice_generic: String,
    /// The mandated reverse-charge mention itself, printed as the headline above the
    /// citation. Directive art. 226(11a) and its national transpositions prescribe the
    /// *words* ("Autoliquidation", "Btw verlegd", …), so this is the one headline whose
    /// wording is fixed by statute rather than chosen.
    pub reverse_charge_headline: String,
    /// The same headline for a seller with no VAT system, which must not be handed a
    /// mention asserting VAT.
    pub reverse_charge_headline_generic: String,
}

/// Statutory mentions resolved for a seller jurisdiction, in the document's
/// presentation language. Fields mirror the resolved translation keys the Typst
/// templates consume.
pub struct LegalMentions {
    pub vat_not_applicable_legal: String,
    /// Headline for the exemption branch; never the place-of-supply one.
    pub vat_exempt_legal: String,
    pub export_notice_resolved: String,
    /// Citation under an out-of-scope row that never left the seller's territory:
    /// CGI art. 294-1, the part of it where VAT does not apply.
    pub no_vat_territory_notice_resolved: String,
    pub vat_exempt_notice_resolved: String,
    pub seller_tax_label: String,
    pub customer_tax_label: String,
    /// Label for an engine-resolved VAT row in the breakdown table. Kept beside the
    /// statutory mentions rather than read raw from the bundle, because it is the one
    /// row label that is jurisdiction-sensitive on top of language-sensitive.
    pub vat_name_resolved: String,
    pub tax_included_resolved: String,
    pub reverse_charge_notice_resolved: String,
    /// Headline above the reverse-charge citation. Resolved rather than read straight
    /// from the bundle: it is a tax-treatment statement like every other headline, and
    /// taking it raw was the one path by which a non-VAT seller could be handed
    /// VAT-worded text.
    pub reverse_charge_headline_resolved: String,
}

impl LegalMentions {
    /// Select the mentions for `jurisdiction` from `candidates`. A national regime
    /// wins first (France cites the CGI, Belgium the Code de la TVA, Switzerland the
    /// LTVA), then VAT vs. non-VAT wording, then EU vs. non-EU.
    pub fn resolve(
        jurisdiction: TaxJurisdiction,
        customer: CustomerKind,
        candidates: LegalCandidates,
    ) -> Self {
        let c = candidates;

        // National citations and the headline over them are VAT articles, so a
        // jurisdiction that does not operate VAT never reaches them even when it
        // carries a regime. Registration-number labels are not: a non-VAT regime
        // (Brazil) contributes exactly those and nothing else.
        let (
            national_export,
            national_seller_label,
            national_customer_label,
            national_headline,
            national_vat_name,
        ) = match c.national.select(jurisdiction.legal) {
            Some(n) if jurisdiction.uses_vat => {
                let export = match customer {
                    CustomerKind::Business => n.export_notice_b2b,
                    CustomerKind::Consumer => n.export_notice_b2c,
                };
                (
                    export,
                    n.seller_tax_label,
                    n.customer_tax_label,
                    n.vat_not_applicable,
                    n.vat_name,
                )
            }
            Some(n) => (None, n.seller_tax_label, n.customer_tax_label, None, None),
            None => (None, None, None, None, None),
        };

        LegalMentions {
            vat_not_applicable_legal: national_headline.unwrap_or(if jurisdiction.uses_vat {
                c.vat_not_applicable
            } else {
                c.tax_not_applicable
            }),
            vat_exempt_legal: if jurisdiction.uses_vat {
                c.vat_exempt_headline
            } else {
                c.tax_exempt_headline
            },
            export_notice_resolved: national_export.unwrap_or({
                // `uses_vat` is checked on the EU arm too. Operating VAT is a condition
                // of membership so the combination cannot arise from `TaxJurisdiction::of`,
                // but the struct's fields are public and a caller that built one by hand
                // would otherwise get "Tax not applicable" over "Not subject to VAT".
                match (jurisdiction.in_eu, jurisdiction.uses_vat, customer) {
                    (true, true, CustomerKind::Business) => c.export_notice_generic,
                    (true, true, CustomerKind::Consumer) => c.export_notice_generic_b2c,
                    (false, true, _) => c.export_notice_non_eu,
                    (_, false, _) => c.export_notice_no_vat,
                }
            }),
            // Not selected by jurisdiction: the engine records this position only for
            // a supply inside France's VAT territory, so French law is the one that
            // answers it whatever regime the seller resolves to — a Guyane seller's
            // own jurisdiction operates no VAT, and art. 294-1 is precisely why.
            no_vat_territory_notice_resolved: c.no_vat_territory_notice,
            vat_exempt_notice_resolved: if jurisdiction.uses_vat {
                c.vat_exempt_notice
            } else {
                c.tax_exempt_notice_generic
            },
            seller_tax_label: national_seller_label.unwrap_or(if jurisdiction.uses_vat {
                c.seller_vat_id
            } else {
                c.seller_tax_id
            }),
            customer_tax_label: national_customer_label.unwrap_or(if jurisdiction.uses_vat {
                c.customer_vat_id
            } else {
                c.customer_tax_id
            }),
            // No `uses_vat` fallback here: the label only ever reaches the page under
            // a row the engine actually resolved as VAT, and a VAT that was charged is
            // named accurately whatever the seller's own system calls its taxes.
            vat_name_resolved: national_vat_name.unwrap_or(c.vat_name),
            tax_included_resolved: if jurisdiction.uses_vat {
                c.tax_included_text
            } else {
                c.tax_included_generic
            },
            reverse_charge_headline_resolved: if jurisdiction.uses_vat {
                c.reverse_charge_headline
            } else {
                c.reverse_charge_headline_generic
            },
            reverse_charge_notice_resolved: if jurisdiction.in_eu {
                c.reverse_charge_notice
            } else {
                c.reverse_charge_notice_generic
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common_domain::country::{EU_MEMBERS, LegalRegime};

    /// Marks every candidate that may only ever reach an EU seller's document.
    const EU_ONLY: &str = "EU-ONLY:";

    /// Sentinel candidates: each field is its own name, so a resolved value tells
    /// us exactly which candidate the corpus selected.
    fn sentinels() -> LegalCandidates {
        fn national(regime: &str) -> NationalCitations {
            NationalCitations {
                export_notice_b2b: Some(format!("export_notice_{regime}")),
                export_notice_b2c: Some(format!("export_notice_{regime}_b2c")),
                ..Default::default()
            }
        }

        /// A regime with a business citation but no settled consumer one, as Italy has.
        fn national_b2b_only(regime: &str) -> NationalCitations {
            NationalCitations {
                export_notice_b2c: None,
                vat_not_applicable: Some(format!("vat_not_applicable_{regime}")),
                ..national(regime)
            }
        }

        /// A regime that also renames the registration number, the headline above
        /// the citation, and the VAT row label, as Switzerland does.
        fn national_with_labels(regime: &str) -> NationalCitations {
            NationalCitations {
                seller_tax_label: Some(format!("seller_tax_label_{regime}")),
                customer_tax_label: Some(format!("customer_tax_label_{regime}")),
                vat_not_applicable: Some(format!("vat_not_applicable_{regime}")),
                vat_name: Some(format!("vat_name_{regime}")),
                ..national(regime)
            }
        }

        /// A regime that contributes labels but no citation, as Austria does.
        fn labels_only(regime: &str) -> NationalCitations {
            NationalCitations {
                seller_tax_label: Some(format!("seller_tax_label_{regime}")),
                customer_tax_label: Some(format!("customer_tax_label_{regime}")),
                ..Default::default()
            }
        }
        LegalCandidates {
            national: NationalCandidates {
                france: national("fr"),
                belgium: national("be"),
                switzerland: national_with_labels("ch"),
                italy: national_b2b_only("it"),
                austria: labels_only("at"),
                portugal: national_with_labels("pt"),
                // Brazil ships only labels in production; the sentinel gives it a
                // citation and a headline too, so the tests can prove the VAT gate
                // holds even if a bundle were to hand a non-VAT regime one.
                brazil: national_with_labels("br"),
            },
            vat_not_applicable: "vat_not_applicable".into(),
            tax_not_applicable: "tax_not_applicable".into(),
            vat_exempt_headline: "vat_exempt_headline".into(),
            tax_exempt_headline: "tax_exempt_headline".into(),
            export_notice_generic: format!("{EU_ONLY}export_notice_generic"),
            export_notice_generic_b2c: format!("{EU_ONLY}export_notice_generic_b2c"),
            export_notice_non_eu: "export_notice_non_eu".into(),
            export_notice_no_vat: "export_notice_no_vat".into(),
            no_vat_territory_notice: "no_vat_territory_notice".into(),
            vat_exempt_notice: "vat_exempt_notice".into(),
            tax_exempt_notice_generic: "tax_exempt_notice_generic".into(),
            seller_vat_id: "seller_vat_id".into(),
            seller_tax_id: "seller_tax_id".into(),
            customer_vat_id: "customer_vat_id".into(),
            customer_tax_id: "customer_tax_id".into(),
            vat_name: "vat_name".into(),
            tax_included_text: "tax_included_text".into(),
            tax_included_generic: "tax_included_generic".into(),
            reverse_charge_notice: format!("{EU_ONLY}reverse_charge_notice"),
            reverse_charge_notice_generic: "reverse_charge_notice_generic".into(),
            // Not EU-only: the mandated reverse-charge word is a VAT concept, not a
            // directive one — a British or Norwegian seller prints it too. It is gated
            // on `uses_vat`, so only a non-VAT seller must never see it.
            reverse_charge_headline: "reverse_charge_headline".into(),
            reverse_charge_headline_generic: "reverse_charge_headline_generic".into(),
        }
    }

    fn resolve(country: &str, customer: CustomerKind) -> LegalMentions {
        LegalMentions::resolve(TaxJurisdiction::of(Some(country)), customer, sentinels())
    }

    fn b2b(country: &str) -> LegalMentions {
        resolve(country, CustomerKind::Business)
    }

    #[test]
    fn france_cites_national_regime() {
        let m = b2b("FR");
        assert_eq!(m.vat_not_applicable_legal, "vat_not_applicable");
        assert_eq!(m.export_notice_resolved, "export_notice_fr");
        assert_eq!(m.vat_exempt_notice_resolved, "vat_exempt_notice");
        assert_eq!(m.seller_tax_label, "seller_vat_id");
        assert_eq!(m.customer_tax_label, "customer_vat_id");
        assert_eq!(m.tax_included_resolved, "tax_included_text");
    }

    #[test]
    fn belgium_cites_its_own_regime_not_the_eu_generic() {
        let m = b2b("BE");
        assert_eq!(m.vat_not_applicable_legal, "vat_not_applicable");
        assert_eq!(m.export_notice_resolved, "export_notice_be");
        assert_eq!(m.seller_tax_label, "seller_vat_id");
    }

    /// Switzerland is a national regime *and* outside the EU: its own citations win,
    /// and nothing EU-directive-worded may leak in — including the reverse-charge
    /// citation, which no jurisdiction branch used to guard.
    #[test]
    fn switzerland_cites_its_own_regime_outside_the_eu() {
        let m = b2b("CH");
        // Swiss law calls this "not taxable", not "VAT not applicable", and the
        // number is an MWST-Nr. rather than the EU VAT identification number.
        assert_eq!(m.vat_not_applicable_legal, "vat_not_applicable_ch");
        assert_eq!(m.seller_tax_label, "seller_tax_label_ch");
        assert_eq!(m.customer_tax_label, "customer_tax_label_ch");
        // The row label follows the seller too: Swiss German writes MWST, not USt.
        assert_eq!(m.vat_name_resolved, "vat_name_ch");
        assert_eq!(m.export_notice_resolved, "export_notice_ch");
        assert_eq!(
            m.reverse_charge_notice_resolved,
            "reverse_charge_notice_generic"
        );
    }

    #[test]
    fn generic_eu_uses_directive_wording_no_national_citation() {
        let m = b2b("DE");
        assert_eq!(m.vat_not_applicable_legal, "vat_not_applicable");
        assert_eq!(m.vat_name_resolved, "vat_name");
        assert_eq!(m.export_notice_resolved, "EU-ONLY:export_notice_generic");
        assert_eq!(
            m.reverse_charge_notice_resolved,
            "EU-ONLY:reverse_charge_notice"
        );
    }

    /// The place-of-supply article differs for a private customer, so a B2C supply may
    /// never inherit the business citation.
    #[test]
    fn consumer_supplies_take_their_own_article() {
        assert_eq!(b2b("FR").export_notice_resolved, "export_notice_fr");
        assert_eq!(
            resolve("FR", CustomerKind::Consumer).export_notice_resolved,
            "export_notice_fr_b2c"
        );
        assert_eq!(
            resolve("DE", CustomerKind::Consumer).export_notice_resolved,
            "EU-ONLY:export_notice_generic_b2c"
        );
    }

    /// A regime with no settled B2C citation falls through to the generic wording
    /// rather than reusing its own business article.
    #[test]
    fn regime_without_a_consumer_citation_falls_through() {
        let mut candidates = sentinels();
        candidates.national.belgium.export_notice_b2c = None;
        let m = LegalMentions::resolve(
            TaxJurisdiction::of(Some("BE")),
            CustomerKind::Consumer,
            candidates,
        );
        assert_eq!(
            m.export_notice_resolved,
            "EU-ONLY:export_notice_generic_b2c"
        );
        // The exemption headline is not place-of-supply dependent, so it still stands.
        assert_eq!(m.vat_not_applicable_legal, "vat_not_applicable");
    }

    #[test]
    fn non_eu_vat_country_has_no_eu_export_mention() {
        let m = b2b("GB");
        assert_eq!(m.vat_not_applicable_legal, "vat_not_applicable");
        assert_eq!(m.export_notice_resolved, "export_notice_non_eu");
        assert_eq!(m.seller_tax_label, "seller_vat_id");
    }

    /// The headline and the citation under it must speak one tax vocabulary: a seller
    /// with no VAT system said "Tax not applicable" over "Not subject to VAT - export".
    #[test]
    fn a_non_vat_seller_is_never_told_about_vat() {
        let m = b2b("US");
        assert_eq!(m.vat_not_applicable_legal, "tax_not_applicable");
        assert_eq!(m.export_notice_resolved, "export_notice_no_vat");
        assert_eq!(m.vat_exempt_notice_resolved, "tax_exempt_notice_generic");
        assert_eq!(m.seller_tax_label, "seller_tax_id");
        assert_eq!(m.customer_tax_label, "customer_tax_id");
        assert_eq!(m.tax_included_resolved, "tax_included_generic");
        assert_eq!(
            m.reverse_charge_headline_resolved,
            "reverse_charge_headline_generic"
        );
    }

    /// A country that carries a national regime but does not operate VAT (Guyane,
    /// Mayotte) must not reach that regime's VAT articles.
    #[test]
    fn a_non_vat_jurisdiction_never_reaches_national_vat_citations() {
        for territory in ["GF", "YT"] {
            let m = b2b(territory);
            assert_eq!(
                m.export_notice_resolved, "export_notice_no_vat",
                "{territory}"
            );
            assert_eq!(m.seller_tax_label, "seller_tax_id", "{territory}");
        }
    }

    /// Monaco bills French VAT from inside the EU VAT area; Liechtenstein bills Swiss
    /// VAT from outside it.
    #[test]
    fn territories_follow_the_regime_they_belong_to() {
        let mc = b2b("MC");
        assert_eq!(mc.export_notice_resolved, "export_notice_fr");

        let li = b2b("LI");
        assert_eq!(li.export_notice_resolved, "export_notice_ch");
    }

    /// Italy carries its own regime for the business case, but has no settled
    /// consumer citation — that must fall through rather than reuse the business
    /// article, which is a different provision.
    #[test]
    fn italy_cites_its_own_regime_for_business_only() {
        assert_eq!(b2b("IT").export_notice_resolved, "export_notice_it");
        assert_eq!(
            resolve("IT", CustomerKind::Consumer).export_notice_resolved,
            "EU-ONLY:export_notice_generic_b2c"
        );
        // Art. 21 comma 6-bis DPR 633/1972 mandates the annotation itself, so Italy
        // carries its own headline rather than the generic one.
        assert_eq!(b2b("IT").vat_not_applicable_legal, "vat_not_applicable_it");
        // Italy is an EU member, so nothing about its regime changes the tax vocabulary.
        assert_eq!(b2b("IT").seller_tax_label, "seller_vat_id");
    }

    /// Austria contributes no citation at all, only its UID-Nummer label: the export
    /// mention must still come from the EU generic, not go missing.
    #[test]
    fn a_regime_with_labels_but_no_citation_keeps_the_generic_mention() {
        let m = b2b("AT");
        assert_eq!(m.export_notice_resolved, "EU-ONLY:export_notice_generic");
        assert_eq!(m.seller_tax_label, "seller_tax_label_at");
        assert_eq!(m.customer_tax_label, "customer_tax_label_at");
        // No headline override, so the generic one still applies.
        assert_eq!(m.vat_not_applicable_legal, "vat_not_applicable");
    }

    /// Liechtenstein bills under the Swiss regime, so it must inherit the Swiss
    /// headline and MWST-Nr. label too — not just the citation.
    #[test]
    fn liechtenstein_inherits_the_whole_swiss_presentation() {
        let li = b2b("LI");
        assert_eq!(li.export_notice_resolved, "export_notice_ch");
        assert_eq!(li.vat_not_applicable_legal, "vat_not_applicable_ch");
        assert_eq!(li.seller_tax_label, "seller_tax_label_ch");
        assert_eq!(li.vat_name_resolved, "vat_name_ch");
    }

    /// A territory that does not operate VAT and carries no regime of its own must
    /// never be handed a national headline or label either: nothing about the French
    /// regime it belongs to for other purposes may reach it.
    #[test]
    fn a_non_vat_jurisdiction_reaches_no_national_presentation() {
        for territory in ["GF", "YT"] {
            let m = b2b(territory);
            assert_eq!(
                m.vat_not_applicable_legal, "tax_not_applicable",
                "{territory}"
            );
            assert_eq!(m.seller_tax_label, "seller_tax_id", "{territory}");
        }
    }

    /// Portugal is an EU member with a full regime: its own article for each supply
    /// basis and its own registration-number label, under the generic EU headline.
    #[test]
    fn portugal_cites_its_own_regime_for_both_supply_bases() {
        let m = b2b("PT");
        assert_eq!(m.export_notice_resolved, "export_notice_pt");
        assert_eq!(
            resolve("PT", CustomerKind::Consumer).export_notice_resolved,
            "export_notice_pt_b2c"
        );
        assert_eq!(m.seller_tax_label, "seller_tax_label_pt");
        // Portugal mandates the *reason*, not a fixed headline, so the directive
        // reverse-charge citation and the VAT vocabulary are the EU generic ones.
        assert_eq!(
            m.reverse_charge_notice_resolved,
            "EU-ONLY:reverse_charge_notice"
        );
        assert_eq!(m.tax_included_resolved, "tax_included_text");
    }

    /// Brazil carries a regime without operating VAT. Its label must reach the
    /// document — that is the whole point of the regime — while the citation and
    /// headline the sentinel gives it must not, because those assert a VAT that does
    /// not exist there.
    #[test]
    fn a_non_vat_regime_contributes_labels_but_never_vat_citations() {
        for customer in [CustomerKind::Business, CustomerKind::Consumer] {
            let m = resolve("BR", customer);
            assert_eq!(m.seller_tax_label, "seller_tax_label_br");
            assert_eq!(m.customer_tax_label, "customer_tax_label_br");
            assert_eq!(m.export_notice_resolved, "export_notice_no_vat");
            assert_eq!(m.vat_not_applicable_legal, "tax_not_applicable");
            assert_eq!(m.vat_exempt_notice_resolved, "tax_exempt_notice_generic");
            assert_eq!(m.tax_included_resolved, "tax_included_generic");
            // The sentinel hands Brazil a national VAT name; the VAT gate must keep
            // it from resolving — Brazil has no VAT for the label to name.
            assert_eq!(m.vat_name_resolved, "vat_name");
            assert_eq!(
                m.reverse_charge_headline_resolved,
                "reverse_charge_headline_generic"
            );
        }
    }

    /// The headline and the citation under it must speak one tax vocabulary even for a
    /// jurisdiction assembled by hand: `TaxJurisdiction`'s fields are public, so the
    /// "EU member without VAT" state is constructible even though `of` never yields it.
    #[test]
    fn a_hand_built_eu_jurisdiction_without_vat_stays_consistent() {
        let jurisdiction = TaxJurisdiction {
            uses_vat: false,
            in_eu: true,
            legal: LegalRegime::Generic,
        };
        let m = LegalMentions::resolve(jurisdiction, CustomerKind::Business, sentinels());
        assert_eq!(m.vat_not_applicable_legal, "tax_not_applicable");
        assert_eq!(m.export_notice_resolved, "export_notice_no_vat");
    }

    /// The invariant the whole EU/non-EU split exists to hold: no seller outside the
    /// EU may be handed directive-worded text, whatever their regime, VAT status or
    /// customer. Swept over every ISO country so a new entry in any table cannot
    /// quietly open a path.
    #[test]
    fn non_eu_seller_never_sees_eu_wording() {
        for country in rust_iso3166::ALL_ALPHA2 {
            for customer in [CustomerKind::Business, CustomerKind::Consumer] {
                let jurisdiction = TaxJurisdiction::of(Some(country));
                if jurisdiction.in_eu {
                    continue;
                }
                let m = LegalMentions::resolve(jurisdiction, customer, sentinels());
                for (field, value) in [
                    ("vat_not_applicable_legal", &m.vat_not_applicable_legal),
                    ("export_notice_resolved", &m.export_notice_resolved),
                    ("vat_exempt_notice_resolved", &m.vat_exempt_notice_resolved),
                    ("tax_included_resolved", &m.tax_included_resolved),
                    (
                        "reverse_charge_notice_resolved",
                        &m.reverse_charge_notice_resolved,
                    ),
                    (
                        "reverse_charge_headline_resolved",
                        &m.reverse_charge_headline_resolved,
                    ),
                ] {
                    assert!(
                        !value.contains(EU_ONLY),
                        "{country} is not in the EU but got EU wording in {field}: {value}"
                    );
                }
            }
        }
        // The sweep only means something if EU sellers really do get the EU strings.
        for country in EU_MEMBERS {
            let m = b2b(country);
            assert!(
                m.reverse_charge_notice_resolved.contains(EU_ONLY),
                "{country} is an EU member and should carry the directive reverse-charge wording"
            );
        }
    }
}
