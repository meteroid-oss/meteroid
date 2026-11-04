use serde::Serialize;
use std::error::Error;
use std::fmt::Display;

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(
    feature = "diesel",
    derive(diesel::expression::AsExpression, diesel::deserialize::FromSqlRow)
)]
#[cfg_attr(feature = "diesel", diesel(sql_type = diesel::sql_types::Text))]
pub struct CountryCode {
    pub code: String,
    pub name: String,
}

impl Default for CountryCode {
    fn default() -> Self {
        CountryCode {
            code: rust_iso3166::FR.alpha2.to_string(),
            name: rust_iso3166::FR.name.to_string(),
        }
    }
}

pub struct Subdivision {
    pub code: String,
    pub name: String,
}

#[cfg(feature = "diesel")]
impl<B: diesel::backend::Backend> diesel::serialize::ToSql<diesel::sql_types::Text, B>
    for CountryCode
where
    String: diesel::serialize::ToSql<diesel::sql_types::Text, B>,
{
    fn to_sql<'b>(
        &'b self,
        out: &mut diesel::serialize::Output<'b, '_, B>,
    ) -> diesel::serialize::Result {
        <String as diesel::serialize::ToSql<diesel::sql_types::Text, B>>::to_sql(&self.code, out)
    }
}
#[cfg(feature = "diesel")]
impl<B: diesel::backend::Backend> diesel::deserialize::FromSql<diesel::sql_types::Text, B>
    for CountryCode
where
    String: diesel::deserialize::FromSql<diesel::sql_types::Text, B>,
{
    fn from_sql(bytes: B::RawValue<'_>) -> diesel::deserialize::Result<Self> {
        let code =
            <String as diesel::deserialize::FromSql<diesel::sql_types::Text, B>>::from_sql(bytes)?;
        Ok(CountryCode::parse_as_opt(&code)
            // soft failure on decode
            .unwrap_or(CountryCode {
                code: "00".to_string(),
                name: format!("Unknown code : {}", &code).to_string(),
            }))
    }
}

#[cfg(feature = "utoipa")]
impl utoipa::PartialSchema for CountryCode {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::Schema> {
        utoipa::openapi::schema::Object::builder()
            .schema_type(utoipa::openapi::schema::SchemaType::Type(
                utoipa::openapi::Type::String,
            ))
            .format(Some(utoipa::openapi::schema::SchemaFormat::Custom(
                "CountryCode".to_string(),
            )))
            .examples(["US", "GB", "FR"])
            .into()
    }
}

#[cfg(feature = "utoipa")]
impl utoipa::ToSchema for CountryCode {
    fn name() -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("CountryCode")
    }
}

impl CountryCode {
    pub fn parse_as_opt(code: &str) -> Option<Self> {
        rust_iso3166::from_alpha2(&code.to_uppercase()).map(std::convert::Into::into)
    }

    pub fn subdivisions(&self) -> Vec<Subdivision> {
        rust_iso3166::from_alpha2(&self.code)
            .map(|s| {
                s.subdivisions()
                    .unwrap_or_default()
                    .iter()
                    .map(|sub| Subdivision {
                        code: sub.code.to_string(),
                        name: sub.name.to_string(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    #[cfg(feature = "tonic")]
    pub fn as_proto(&self) -> String {
        self.code.clone()
    }

    #[cfg(feature = "tonic")]
    pub fn from_proto<T: AsRef<str>>(value: T) -> Result<Self, tonic::Status> {
        Self::parse_as_opt(value.as_ref()).ok_or(tonic::Status::invalid_argument(format!(
            "Invalid country code: {}",
            value.as_ref()
        )))
    }

    #[cfg(feature = "tonic")]
    pub fn from_proto_opt<T: AsRef<str>>(value: Option<T>) -> Result<Option<Self>, tonic::Status> {
        value.map(Self::from_proto).transpose()
    }
}

impl From<rust_iso3166::CountryCode> for CountryCode {
    fn from(cc: rust_iso3166::CountryCode) -> Self {
        Self {
            code: cc.alpha2.to_string(),
            name: cc.name.to_string(),
        }
    }
}

impl std::fmt::Display for CountryCode {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.code)
    }
}

impl std::str::FromStr for CountryCode {
    type Err = CountryCodeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse_as_opt(s).ok_or_else(|| CountryCodeError(format!("Invalid country code: {s}")))
    }
}

impl<'de> serde::Deserialize<'de> for CountryCode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(CountryCode::parse_as_opt(&s)
            // soft failure on decode
            .unwrap_or(CountryCode {
                code: "00".to_string(),
                name: format!("Unknown code : {}", &s).to_string(),
            }))
    }
}

impl Serialize for CountryCode {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.code)
    }
}

#[derive(Debug)]
pub struct CountryCodeError(pub(crate) String);
impl Display for CountryCodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "CountryCodeError: {}", self.0)
    }
}

impl Error for CountryCodeError {}

/// EU-27 member states by ISO alpha-2. Single source of truth for EU membership.
pub const EU_MEMBERS: &[&str] = &[
    "AT", "BE", "BG", "HR", "CY", "CZ", "DK", "EE", "FI", "FR", "DE", "GR", "HU", "IE", "IT", "LV",
    "LT", "LU", "MT", "NL", "PL", "PT", "RO", "SK", "SI", "ES", "SE",
];

/// VAT-operating countries outside the EU.
///
/// Presentation only: this decides whether a document says "VAT" or generic "tax",
/// and whether a tax ID is labelled a VAT number. It is deliberately *not* derived
/// from the tax engine's rate database — that database tags any ad-valorem
/// consumption tax `"vat"` (Jersey's GST, Puerto Rico's sales-and-use tax, US state
/// sales taxes), which would print "VAT" on documents for countries that have none.
///
/// Membership is "operates a tax that is a VAT in substance", verified against each
/// administration's own naming: a multi-stage credit-invoice tax on value added.
/// Excluded on that test, deliberately: Jersey and Guernsey (GST), Andorra (IGI),
/// San Marino (single-stage import tax), Gibraltar (no general consumption tax),
/// the Canaries (IGIC) and Ceuta/Melilla (IPSI) — none of which is a VAT, however
/// a rate table may label them.
#[rustfmt::skip]
pub const NON_EU_VAT_COUNTRIES: &[&str] = &[
    // Western Europe.
    "GB", "CH", "NO", "IS",
    // Isle of Man: inside the UK VAT territory, Manx VAT parallels UK law.
    "IM",
    // Faroe Islands: meirvirðisgjald (MVG).
    "FO",
    // Western Balkans and Türkiye.
    "TR", "RS", "AL", "MK", "BA", "ME",
    // Eastern Europe and the Caucasus.
    "UA", "MD", "RU", "GE", "AM", "AZ",
    // French Pacific collectivités with their own VAT-type taxes: French Polynesia
    // levies a local TVA, New Caledonia the TGC ("une taxe de type TVA").
    "PF", "NC",
    // Brazil is deliberately absent. The 2023 reform (EC 132/2023, LC 214/2025)
    // creates a dual VAT — the federal CBS and the subnational IBS — but 2026 is a
    // test year at nominal rates, with the taxes it replaces (PIS/COFINS, ICMS, ISS)
    // still the ones a document actually carries. Documents therefore keep the
    // generic "tax" vocabulary until the transition makes "IVA" the operative tax;
    // the rate database's `vat` tag for BR is the ICMS approximation, not a VAT.
];

pub fn is_eu_member(alpha2: &str) -> bool {
    EU_MEMBERS.contains(&alpha2)
}

/// The VAT territory whose law governs a country's supplies, as an alpha-2 code.
///
/// Most countries are their own territory. The exceptions are territories folded into
/// a neighbour's VAT area by treaty or directive, where a supply *between* the two is
/// domestic rather than cross-border — the distinction `TaxJurisdiction` cannot carry,
/// because it describes one party in isolation while this compares two.
///
/// Callers use it to decide whether a supply leaves the seller's VAT territory at all.
/// It says nothing about the *rate*: Monaco, the Isle of Man and Liechtenstein apply
/// their host's rates, but the French overseas departments have their own (CGI art. 296)
/// while still not being third territories towards France.
pub fn vat_territory(alpha2: &str) -> &str {
    match alpha2 {
        // Directive art. 7(2): transactions with Monaco are treated as French, and
        // those with the Isle of Man as United Kingdom. Convention franco-monégasque
        // du 18 mai 1963 art. 15 aligns Monegasque rates with France's.
        "MC" => "FR",
        "IM" => "GB",
        // Swiss–Liechtenstein VAT union: "gemeinsames Anwendungsgebiet der
        // Mehrwertsteuer für beide Vertragsstaaten" (Vereinbarung SR 0.641.295.142.1
        // art. 2). Rates move in lockstep.
        "LI" => "CH",
        // The art. 73 collectivités are outside the EU VAT area but, per BOFiP
        // BOI-TVA-GEO-20-40 §230, "ne sont pas considérées comme des territoires
        // tiers" towards France: a service supplied from the mainland carries no
        // export treatment and no reverse charge, the supplier staying liable.
        // Their own rates (CGI art. 296) live in the rate database under their own
        // codes — which is why [`vat_rate_territory`] deliberately does not fold them.
        "GP" | "MQ" | "RE" | "GF" | "YT" => "FR",
        other => other,
    }
}

/// The country whose VAT *rates* apply to a territory, as an alpha-2 code.
///
/// A strict subset of [`vat_territory`]: only territories that adopt their host's rate
/// schedule wholesale, so a scenario may be resolved against the host and get the right
/// number. Monaco applies French rates by Convention franco-monégasque art. 15
/// ("sur les mêmes bases et aux mêmes tarifs qu'en France"), the Isle of Man parallels
/// UK rates, and Liechtenstein moves in lockstep with Switzerland.
///
/// The French overseas departments are deliberately absent: they are inside the French
/// VAT territory for [`vat_territory`] purposes, but CGI art. 296 gives them their own
/// rates (8.5% standard, 2.1% reduced), so resolving them against metropolitan France
/// would charge 20%. They resolve against their own rate-database entries instead, the
/// engine treating a métropole↔DOM supply as domestic at the destination's schedule
/// (BOFiP BOI-TVA-GEO-20-40 §100 and §230).
pub fn vat_rate_territory(alpha2: &str) -> &str {
    match alpha2 {
        "MC" => "FR",
        "IM" => "GB",
        "LI" => "CH",
        other => other,
    }
}

/// Whether a supply between these two countries stays inside one VAT territory.
pub fn same_vat_territory(a: &str, b: &str) -> bool {
    vat_territory(a) == vat_territory(b)
}

/// National legal regime whose statute citations a document must carry. Add a
/// variant (and wire its l10n) when onboarding a country with its own mentions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LegalRegime {
    /// No national statute; generic EU-directive wording only.
    #[default]
    Generic,
    France,
    Belgium,
    Switzerland,
    /// Italy mandates the annotation "operazione non soggetta" with the national
    /// article for supplies outside the scope (DPR 633/1972 art. 21 comma 6-bis),
    /// which the e-invoice already carries as Natura N2.1. A directive-only
    /// citation is short of what an Italian document is expected to say.
    Italy,
    /// Austria contributes no statutory citation of its own — a non-taxable supply
    /// needs no special mention under §11 UStG 1994 — but its registration number is
    /// a UID-Nummer, not the German USt-IdNr. the shared `de` bundle names.
    Austria,
    /// Portugal requires "o motivo justificativo da não aplicação do imposto" on the
    /// invoice itself (CIVA art. 36 n.º 5 e)), which in practice is the national
    /// article: art. 6 n.º 6 a) a contrario for a business customer abroad, art. 6
    /// n.º 9 h) for an electronically supplied service to a consumer abroad. The
    /// registration number is the NIF (art. 36 n.º 5 a)).
    ///
    /// Citations are the smaller half of Portuguese compliance. A Portuguese seller
    /// may only issue invoices from software certified by the Autoridade Tributária
    /// (Portaria n.º 363/2010), carrying a certificate number, an ATCUD and a QR
    /// code; none of that is produced here. This regime makes a document *say* the
    /// right thing, not makes it a valid Portuguese fatura.
    Portugal,
    /// Brazil operates no VAT (see [`NON_EU_VAT_COUNTRIES`]), so it contributes no
    /// citation and never reaches one; it names the seller's registration number the
    /// CNPJ, which is what a Brazilian document is expected to print.
    ///
    /// As with Portugal, the fiscal document itself is not produced here: a
    /// Brazilian sale is documented by an NF-e or NFS-e issued through the state or
    /// municipal system. This document is the commercial *fatura* that accompanies
    /// it.
    Brazil,
}

/// How a seller's country is presented on tax documents. Resolved once from the
/// seller country; callers branch on these capabilities, never on country codes.
/// Adding a country means editing the tables above and, for a national regime,
/// `LegalRegime` — nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaxJurisdiction {
    /// Uses VAT nomenclature ("VAT"/"TVA") and EU-style mentions; false for
    /// sales-tax / GST / no-tax systems (US, CA, …), which read as generic "tax".
    pub uses_vat: bool,
    /// EU member: the EU VAT Directive compliance mentions apply.
    pub in_eu: bool,
    /// National statute citations to render for exemptions.
    pub legal: LegalRegime,
}

impl TaxJurisdiction {
    pub fn of(alpha2: Option<&str>) -> Self {
        // Callers pass a `CountryCode.code`, which `parse_as_opt` already uppercased,
        // but the signature takes any `&str` and every table below is uppercase.
        let code = &alpha2.unwrap_or_default().to_ascii_uppercase();
        let code = code.as_str();
        if let Some(territory) = Self::of_territory(code) {
            return territory;
        }
        let in_eu = is_eu_member(code);
        Self {
            uses_vat: in_eu || NON_EU_VAT_COUNTRIES.contains(&code),
            in_eu,
            legal: match code {
                "FR" => LegalRegime::France,
                "BE" => LegalRegime::Belgium,
                "CH" => LegalRegime::Switzerland,
                "IT" => LegalRegime::Italy,
                "AT" => LegalRegime::Austria,
                "PT" => LegalRegime::Portugal,
                "BR" => LegalRegime::Brazil,
                _ => LegalRegime::Generic,
            },
        }
    }

    /// Territories whose VAT treatment does not follow from their own country code,
    /// because they sit inside another country's VAT area, outside the EU VAT area,
    /// or both. Directive 2006/112/EC art. 6 excludes several member-state territories
    /// from the EU VAT area, and art. 7(2) folds Monaco into France's.
    ///
    /// Art. 6(1)(c) no longer names the overseas departments one by one: Directive
    /// 2013/61/EU replaced the list with "the French territories referred to in
    /// Article 349 and Article 355(1) TFEU", deliberately, so that Mayotte stays
    /// covered "regardless of any change in their status under French law".
    ///
    /// Sub-national exclusions (Livigno, Campione d'Italia, Büsingen, Heligoland,
    /// Mount Athos) have no ISO 3166-1 code at all and are not resolved here — they
    /// would need the entity's subdivision. The Canaries and Ceuta/Melilla do have
    /// codes (`IC`, `EA`), but only as *exceptionally reserved* ones, which
    /// `rust_iso3166::from_alpha2` rejects, so they cannot reach this table either.
    /// `XI` (Northern Ireland) is not an ISO code in any register and must never
    /// reach a country-code field; NI services follow plain UK VAT under `GB`.
    fn of_territory(code: &str) -> Option<Self> {
        let jurisdiction = |uses_vat, in_eu, legal| Self {
            uses_vat,
            in_eu,
            legal,
        };
        Some(match code {
            // Inside the French VAT territory: French VAT, French mentions.
            "MC" => jurisdiction(true, true, LegalRegime::France),
            // French overseas departments charge French VAT but are third territories
            // towards the EU. They take the neutral non-EU export wording, not the French
            // regime: the CGI export citation embeds the EU VAT Directive, which must not
            // appear on a document the seller issues from outside the EU VAT area.
            //
            // Their own rates (CGI art. 296: 8.5% standard, 2.1% reduced) are carried in
            // the rate database under their own codes, and the engine rates a
            // métropole↔DOM supply as domestic at the destination's schedule — never as
            // an export and never reverse-charged (BOFiP BOI-TVA-GEO-20-40 §230).
            "GP" | "MQ" | "RE" => jurisdiction(true, false, LegalRegime::Generic),
            // Guyane and Mayotte: VAT is not applicable at all (art. 294 CGI), so the
            // French VAT citations must not be reached either. Note this is about the
            // *rate*, not third-territory status: like the other art. 73 collectivités
            // they are not third territories towards France, which is why
            // [`vat_territory`] still folds them into `FR`.
            "GF" | "YT" => jurisdiction(false, false, LegalRegime::Generic),
            // Åland applies Finnish VAT but is outside the EU VAT area.
            "AX" => jurisdiction(true, false, LegalRegime::Generic),
            // Liechtenstein forms a VAT union with Switzerland.
            "LI" => jurisdiction(true, false, LegalRegime::Switzerland),
            _ => return None,
        })
    }
}

#[cfg(test)]
mod jurisdiction_tests {
    use super::{EU_MEMBERS, LegalRegime, TaxJurisdiction, same_vat_territory, vat_territory};

    #[test]
    fn resolves_seller_presentation_by_country() {
        let fr = TaxJurisdiction::of(Some("FR"));
        assert!(fr.uses_vat && fr.in_eu);
        assert_eq!(fr.legal, LegalRegime::France);

        let de = TaxJurisdiction::of(Some("DE"));
        assert!(de.uses_vat && de.in_eu);
        assert_eq!(de.legal, LegalRegime::Generic);

        let be = TaxJurisdiction::of(Some("BE"));
        assert!(be.uses_vat && be.in_eu);
        assert_eq!(be.legal, LegalRegime::Belgium);

        // Non-EU VAT: VAT terminology, no EU-directive mentions. Switzerland still has
        // a national regime of its own, the others don't.
        for c in ["GB", "CH", "NO"] {
            let j = TaxJurisdiction::of(Some(c));
            assert!(j.uses_vat && !j.in_eu, "{c} should be non-EU VAT");
        }
        assert_eq!(
            TaxJurisdiction::of(Some("CH")).legal,
            LegalRegime::Switzerland
        );
        for c in ["GB", "NO"] {
            assert_eq!(
                TaxJurisdiction::of(Some(c)).legal,
                LegalRegime::Generic,
                "{c}"
            );
        }

        // Non-VAT and unknown default to fully generic (no VAT/EU/national spillover).
        for j in [TaxJurisdiction::of(Some("US")), TaxJurisdiction::of(None)] {
            assert!(!j.uses_vat && !j.in_eu);
            assert_eq!(j.legal, LegalRegime::Generic);
        }
    }

    /// Operating VAT is a condition of EU membership, so no member state may ever
    /// resolve to the generic "tax" vocabulary.
    #[test]
    fn every_eu_member_operates_vat() {
        for c in EU_MEMBERS {
            let j = TaxJurisdiction::of(Some(c));
            assert!(j.uses_vat && j.in_eu, "{c} is an EU member without VAT");
        }
    }

    /// Territories folded into a neighbour's VAT area: a supply between the two is
    /// domestic, so nothing may treat it as leaving the seller's VAT territory.
    #[test]
    fn folded_territories_share_their_host_vat_territory() {
        for (territory, host) in [("MC", "FR"), ("IM", "GB"), ("LI", "CH")] {
            assert_eq!(vat_territory(territory), host, "{territory}");
            assert!(same_vat_territory(territory, host), "{territory}");
            assert!(same_vat_territory(host, territory), "{territory}");
        }
        // The art. 73 collectivités are not third territories towards France
        // (BOFiP BOI-TVA-GEO-20-40 §230), even the two with no VAT of their own.
        for dom in ["GP", "MQ", "RE", "GF", "YT"] {
            assert!(same_vat_territory(dom, "FR"), "{dom}");
        }
        // Everything else is its own territory, and unrelated pairs stay foreign.
        assert_eq!(vat_territory("DE"), "DE");
        assert!(!same_vat_territory("FR", "DE"));
        assert!(!same_vat_territory("CH", "FR"));
        assert!(!same_vat_territory("GB", "FR"));
    }

    /// Territories that operate a VAT must read as VAT, or their documents print
    /// "Tax ID" for a number their administration calls a VAT number.
    #[test]
    fn vat_operating_territories_read_as_vat() {
        for c in ["IM", "FO", "PF", "NC", "IS", "TR", "RS", "UA", "GE"] {
            let j = TaxJurisdiction::of(Some(c));
            assert!(j.uses_vat, "{c} operates a VAT and must read as VAT");
            assert!(!j.in_eu, "{c} is not an EU member");
        }
    }

    /// Taxes that resemble a VAT but are not one: these must keep the generic "tax"
    /// vocabulary however a rate table labels them.
    #[test]
    fn vat_lookalikes_are_not_vat() {
        for c in ["JE", "GG", "AD", "SM", "GI"] {
            assert!(
                !TaxJurisdiction::of(Some(c)).uses_vat,
                "{c} does not operate a VAT"
            );
        }
    }

    /// Italy carries its own regime: DPR 633/1972 art. 21 comma 6-bis expects the
    /// national article, not a directive-only citation.
    #[test]
    fn italy_carries_its_own_regime() {
        let it = TaxJurisdiction::of(Some("IT"));
        assert!(it.uses_vat && it.in_eu);
        assert_eq!(it.legal, LegalRegime::Italy);
    }

    /// GST and sales tax are not VAT: those countries must read as generic "tax", or
    /// an invoice would print VAT wording for a system that has none.
    #[test]
    fn gst_and_sales_tax_countries_are_not_vat() {
        // Portugal is an EU member with its own regime; Brazil carries a regime
        // without operating VAT, which is the combination the corpus has to gate.
        let pt = TaxJurisdiction::of(Some("PT"));
        assert!(pt.uses_vat && pt.in_eu);
        assert_eq!(pt.legal, LegalRegime::Portugal);
        let br = TaxJurisdiction::of(Some("BR"));
        assert!(!br.uses_vat && !br.in_eu);
        assert_eq!(br.legal, LegalRegime::Brazil);

        for c in ["US", "CA", "AU", "NZ", "IN", "SG"] {
            assert!(
                !TaxJurisdiction::of(Some(c)).uses_vat,
                "{c} is not a VAT system"
            );
        }
    }

    #[test]
    fn territories_follow_the_vat_area_they_belong_to() {
        // Monaco is inside the French VAT territory (Directive art. 7(2)).
        let mc = TaxJurisdiction::of(Some("MC"));
        assert!(mc.uses_vat && mc.in_eu);
        assert_eq!(mc.legal, LegalRegime::France);

        // Liechtenstein forms a VAT union with Switzerland, outside the EU.
        let li = TaxJurisdiction::of(Some("LI"));
        assert!(li.uses_vat && !li.in_eu);
        assert_eq!(li.legal, LegalRegime::Switzerland);

        // French overseas departments: French VAT, but third territories towards the EU,
        // so the generic non-EU wording rather than the directive-embedding French regime.
        for c in ["GP", "MQ", "RE"] {
            let j = TaxJurisdiction::of(Some(c));
            assert!(j.uses_vat, "{c} applies French VAT");
            assert!(!j.in_eu, "{c} is outside the EU VAT area");
            assert_eq!(j.legal, LegalRegime::Generic, "{c}");
        }

        // Guyane and Mayotte have no VAT at all (art. 294 CGI), so no VAT regime either.
        for c in ["GF", "YT"] {
            let j = TaxJurisdiction::of(Some(c));
            assert!(!j.uses_vat && !j.in_eu, "{c}");
            assert_eq!(j.legal, LegalRegime::Generic, "{c}");
        }

        // Åland applies Finnish VAT from outside the EU VAT area.
        let ax = TaxJurisdiction::of(Some("AX"));
        assert!(ax.uses_vat && !ax.in_eu);
    }

    /// `of_territory` is consulted before the normal path, so a territory key that was
    /// also an EU member would silently override `is_eu_member` — the documented single
    /// source of truth for EU membership. Keep the two sets disjoint.
    #[test]
    fn territory_overrides_never_shadow_eu_membership() {
        for code in ["MC", "LI", "AX", "GP", "MQ", "RE", "GF", "YT"] {
            assert!(
                !EU_MEMBERS.contains(&code),
                "{code} is both a territory override and an EU member"
            );
        }
    }
}
