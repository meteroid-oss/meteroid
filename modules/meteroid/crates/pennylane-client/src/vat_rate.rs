use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use serde::Serialize;

/// A `vat_rate` code of the customer invoice import: a country's rate (`FR_200` is 20% in France,
/// `FR_1_05` is 1.05%) or a treatment without collected VAT (`exempt`, `crossborder`, `extracom`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct VatRate(&'static str);

impl VatRate {
    /// No VAT collected, the supply being exempt or its position undetermined.
    pub const EXEMPT: VatRate = VatRate("exempt");
    /// An intra-EU supply on which the customer self-accounts for the VAT (reverse charge).
    pub const CROSSBORDER: VatRate = VatRate("crossborder");
    /// A supply outside the scope of the seller's VAT, such as an export.
    pub const EXTRACOM: VatRate = VatRate("extracom");

    pub fn from_code(code: &str) -> Option<VatRate> {
        VAT_RATE_CODES
            .iter()
            .find(|known| **known == code)
            .map(|known| VatRate(known))
    }

    /// The code of `rate` (a fraction: 0.2 for 20%) in the country, when Pennylane has one.
    pub fn standard(country_alpha2: &str, rate: Decimal) -> Option<VatRate> {
        let percent = (rate * Decimal::ONE_HUNDRED).round_dp(3).normalize();
        if percent <= Decimal::ZERO {
            return None;
        }
        let tenths = percent * Decimal::TEN;
        let suffix = if tenths.fract().is_zero() {
            format!("{:02}", tenths.to_u32()?)
        } else {
            percent.to_string().replace('.', "_")
        };
        Self::from_code(&format!("{}_{suffix}", country_alpha2.to_uppercase()))
    }

    pub fn code(&self) -> &'static str {
        self.0
    }
}

/// The `vat_rate` enum of `POST /api/external/v2/customer_invoices/import`, as published.
pub const VAT_RATE_CODES: [&str; 179] = [
    "FR_1_05",
    "FR_1_75",
    "FR_09",
    "FR_21",
    "FR_40",
    "FR_50",
    "FR_55",
    "FR_60",
    "FR_65",
    "FR_85",
    "FR_92",
    "FR_100",
    "FR_130",
    "FR_15_385",
    "FR_160",
    "FR_196",
    "FR_200",
    "AD_10",
    "AD_45",
    "AD_95",
    "AT_100",
    "AT_130",
    "AT_190",
    "AT_200",
    "BE_60",
    "BE_120",
    "BE_210",
    "BG_90",
    "BG_200",
    "CH_25",
    "CH_37",
    "CH_77",
    "CH_26",
    "CH_38",
    "CH_81",
    "CY_30",
    "CY_50",
    "CY_80",
    "CY_90",
    "CY_190",
    "CZ_100",
    "CZ_120",
    "CZ_150",
    "CZ_210",
    "DE_70",
    "DE_190",
    "DK_250",
    "EE_90",
    "EE_200",
    "EE_220",
    "EE_240",
    "ES_40",
    "ES_70",
    "ES_100",
    "ES_210",
    "FI_100",
    "FI_135",
    "FI_140",
    "FI_240",
    "FI_255",
    "GB_50",
    "GB_200",
    "GR_60",
    "GR_130",
    "GR_170",
    "GR_240",
    "GR_40",
    "HR_50",
    "HR_130",
    "HR_250",
    "HU_50",
    "HU_180",
    "HU_270",
    "IE_48",
    "IE_90",
    "IE_135",
    "IE_210",
    "IE_230",
    "IT_40",
    "IT_50",
    "IT_100",
    "IT_220",
    "LT_50",
    "LT_90",
    "LT_120",
    "LT_210",
    "LU_30",
    "LU_70",
    "LU_80",
    "LU_120",
    "LU_130",
    "LU_140",
    "LU_160",
    "LU_170",
    "LV_50",
    "LV_120",
    "LV_210",
    "MC_09",
    "MC_21",
    "MC_55",
    "MC_85",
    "MC_100",
    "MC_200",
    "MT_50",
    "MT_70",
    "MT_120",
    "MT_180",
    "MU_150",
    "NL_90",
    "NL_210",
    "PL_50",
    "PL_80",
    "PL_230",
    "PT_60",
    "PT_130",
    "PT_160",
    "PT_180",
    "PT_220",
    "PT_230",
    "RO_50",
    "RO_90",
    "RO_110",
    "RO_190",
    "RO_210",
    "SE_60",
    "SE_120",
    "SE_250",
    "SI_50",
    "SI_95",
    "SI_220",
    "SK_50",
    "SK_100",
    "SK_190",
    "SK_200",
    "SK_230",
    "NO_120",
    "NO_150",
    "NO_250",
    "DE_70_BU0002",
    "DE_190_BU0003",
    "DE_70_BU0008",
    "DE_190_BU0009",
    "DE_intracom_70_BU0012",
    "DE_intracom_190_BU0013",
    "DE_190_electronic_BU0226",
    "exempt",
    "DE_intracom_exempt_BU0011",
    "DE_intracom_exempt_BU0047",
    "DE_intracom_exempt_BU0233",
    "DE_exempt_BU0001",
    "DE_exempt_BU0181",
    "DE_exempt_BU0490",
    "intracom_21",
    "intracom_55",
    "intracom_85",
    "intracom_100",
    "crossborder",
    "ES_intracom_sales_goods",
    "ES_intracom_sales_services",
    "DE_intracom_70_BU0018",
    "DE_intracom_190_BU0019",
    "DE_intracom_190_BU0506",
    "extracom",
    "DE_extracom_BU0173",
    "DE_extracom_BU0191",
    "DE_extracom_190_BU0511",
    "ES_isp_sales",
    "ES_suplido",
    "FR_85_construction",
    "FR_100_construction",
    "FR_200_construction",
    "DE_exempt_construction_BU0046",
    "DE_70_construction_BU0091",
    "DE_190_construction_BU0094",
    "DE_190_construction_BU0526",
    "DE_190_construction_BU0546",
    "ES_isp_purchases_210",
    "ES_isp_purchases_100",
    "mixed",
];

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn a_rate_finds_its_country_code() {
        let code = |country, rate| VatRate::standard(country, rate).map(|v| v.code());
        assert_eq!(code("FR", dec!(0.2)), Some("FR_200"));
        assert_eq!(code("fr", dec!(0.055)), Some("FR_55"));
        assert_eq!(code("FR", dec!(0.021)), Some("FR_21"));
        assert_eq!(code("FR", dec!(0.009)), Some("FR_09"));
        assert_eq!(code("FR", dec!(0.0105)), Some("FR_1_05"));
        assert_eq!(code("FR", dec!(0.15385)), Some("FR_15_385"));
        assert_eq!(code("DE", dec!(0.19)), Some("DE_190"));
        assert_eq!(code("DE", dec!(0.07)), Some("DE_70"));
        assert_eq!(code("BE", dec!(0.21)), Some("BE_210"));
        assert_eq!(code("CH", dec!(0.081)), Some("CH_81"));
        assert_eq!(code("IE", dec!(0.135)), Some("IE_135"));
        assert_eq!(code("HU", dec!(0.27)), Some("HU_270"));
        assert_eq!(code("GB", dec!(0.2)), Some("GB_200"));
    }

    #[test]
    fn a_rate_is_matched_whatever_its_scale() {
        let code = |rate| VatRate::standard("FR", rate).map(|v| v.code());
        assert_eq!(code(dec!(0.20000)), Some("FR_200"));
        assert_eq!(code(dec!(0.2000001)), Some("FR_200"));
        assert_eq!(code(dec!(0.153846)), Some("FR_15_385"));
    }

    #[test]
    fn a_rate_without_a_code_has_none() {
        assert_eq!(VatRate::standard("FR", dec!(0.19)), None);
        assert_eq!(VatRate::standard("US", dec!(0.2)), None);
        assert_eq!(VatRate::standard("FR", Decimal::ZERO), None);
    }

    #[test]
    fn treatments_are_published_codes() {
        for treatment in [VatRate::EXEMPT, VatRate::CROSSBORDER, VatRate::EXTRACOM] {
            assert_eq!(VatRate::from_code(treatment.code()), Some(treatment));
        }
    }
}
