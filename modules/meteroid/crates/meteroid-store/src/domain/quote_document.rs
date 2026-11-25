//! The content a quote's signers see, in a canonical form whose SHA-256 identifies it. A signature
//! records the hash it was given; a quote edited since then no longer matches.

use chrono::{NaiveDate, NaiveDateTime};
use common_domain::ids::{CouponId, ProductId};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::domain::coupons::{Coupon, CouponDiscount};
use crate::domain::customers::Address;
use crate::domain::plan_minimums::MinimumScope;
use crate::domain::price_components::{RatingModel, UsagePricingModel};
use crate::domain::quotes::{
    DetailedQuote, QuoteAttachment, QuoteCountersigner, QuoteDisplayOptions, QuoteSection,
    RecipientDetails,
};
use crate::domain::{SubscriptionFee, SubscriptionFeeBillingPeriod};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteDocument {
    pub quote_number: String,
    pub revision: i32,
    pub currency: String,
    pub seller: QuoteDocumentParty,
    pub customer: QuoteDocumentParty,
    pub billing_start_date: Option<NaiveDate>,
    pub billing_end_date: Option<NaiveDate>,
    pub trial_duration_days: Option<i32>,
    pub billing_day_anchor: Option<i32>,
    pub expires_at: Option<NaiveDateTime>,
    pub net_terms: i32,
    pub overview: Option<String>,
    pub terms_and_services: Option<String>,
    pub sections: Vec<QuoteSection>,
    pub display_options: QuoteDisplayOptions,
    pub lines: Vec<QuoteDocumentLine>,
    pub coupon_ids: Vec<CouponId>,
    // Absent from documents recorded before the terms were; skipped when empty so a document
    // without coupons keeps the hash it had.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub coupons: Vec<QuoteDocumentCoupon>,
    pub minimum: Option<QuoteDocumentMinimum>,
    pub recipients: Vec<RecipientDetails>,
    pub countersigners: Vec<QuoteCountersigner>,
    /// Their content's hash is part of the document's: a signature covers the files it was given.
    pub attachments: Vec<QuoteAttachment>,
    pub consent_text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteDocumentParty {
    pub name: String,
    pub address: Option<Address>,
    pub vat_number: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteDocumentLine {
    pub name: String,
    pub description: Option<String>,
    pub period: SubscriptionFeeBillingPeriod,
    pub fee: SubscriptionFee,
    pub list_fee: Option<SubscriptionFee>,
    pub discount_percent: Option<Decimal>,
    pub quantity: Option<i32>,
}

/// A coupon's terms as the signers read them; applied to the subscription on conversion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuoteDocumentCoupon {
    pub code: String,
    pub description: Option<String>,
    pub discount: CouponDiscount,
    /// Billing periods the discount applies to; neither this nor `duration_months`: every period.
    pub recurring_value: Option<i32>,
    /// Billing periods starting within this many months of the first paid period. Skipped when
    /// none, so a document recorded before keeps its hash.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_months: Option<i32>,
}

impl From<&Coupon> for QuoteDocumentCoupon {
    fn from(coupon: &Coupon) -> Self {
        let description = coupon.description.trim();
        QuoteDocumentCoupon {
            code: coupon.code.clone(),
            description: (!description.is_empty()).then(|| description.to_string()),
            discount: coupon.discount.clone(),
            recurring_value: coupon.recurring_value,
            duration_months: coupon.duration_months,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteDocumentMinimum {
    pub amount_subunit: i64,
    pub product_ids: Option<Vec<ProductId>>,
    pub pending_product_names: Vec<String>,
}

impl QuoteDocument {
    pub fn from_detailed(quote: &DetailedQuote) -> Self {
        let q = &quote.quote;
        let entity = &quote.invoicing_entity;

        // In the order the document shows them: the components, then the add-ons.
        let mut components: Vec<_> = quote.components.iter().collect();
        components.sort_by_key(|c| c.sort_order);
        let mut add_ons: Vec<_> = quote.add_ons.iter().collect();
        add_ons.sort_by_key(|a| a.sort_order);
        let lines = components
            .into_iter()
            .map(|c| QuoteDocumentLine {
                name: c.name.clone(),
                description: c.description.clone(),
                period: c.period,
                fee: c.fee.clone(),
                list_fee: c.list_fee.clone(),
                discount_percent: c.discount_percent,
                quantity: None,
            })
            .chain(add_ons.into_iter().map(|a| QuoteDocumentLine {
                name: a.name.clone(),
                description: a.description.clone(),
                period: a.period,
                fee: a.fee.clone(),
                list_fee: a.list_fee.clone(),
                discount_percent: a.discount_percent,
                quantity: Some(a.quantity),
            }))
            .collect();

        QuoteDocument {
            quote_number: q.quote_number.clone(),
            revision: q.revision,
            currency: q.currency.clone(),
            seller: QuoteDocumentParty {
                name: entity.legal_name.clone(),
                address: Some(Address {
                    line1: entity.address_line1.clone(),
                    line2: entity.address_line2.clone(),
                    city: entity.city.clone(),
                    country: Some(entity.country.clone()),
                    state: entity.state.clone(),
                    zip_code: entity.zip_code.clone(),
                }),
                vat_number: entity.vat_number.clone(),
            },
            customer: QuoteDocumentParty {
                name: quote.customer.name.clone(),
                address: quote.customer.billing_address.clone(),
                vat_number: quote.customer.vat_number.clone(),
            },
            billing_start_date: q.billing_start_date,
            billing_end_date: q.billing_end_date,
            trial_duration_days: q.trial_duration_days,
            billing_day_anchor: q.billing_day_anchor,
            expires_at: q.expires_at,
            net_terms: q.net_terms,
            overview: q.overview.clone(),
            terms_and_services: q.terms_and_services.clone(),
            sections: q.sections.clone(),
            display_options: q.display_options.clone(),
            lines,
            coupon_ids: quote.coupons.iter().map(|c| c.coupon_id).collect(),
            coupons: quote.coupon_terms(),
            minimum: quote.minimum.as_ref().map(|m| QuoteDocumentMinimum {
                amount_subunit: m.amount_subunit,
                product_ids: match &m.scope {
                    MinimumScope::AllComponents => None,
                    MinimumScope::Products(ids) => Some(ids.clone()),
                },
                pending_product_names: m.pending_extra_product_names.clone(),
            }),
            recipients: q.recipients.clone(),
            countersigners: q.countersigners.clone(),
            attachments: q.attachments.clone(),
            consent_text: q.consent_text.clone(),
        }
    }

    pub fn hash(&self) -> String {
        let bytes = serde_json::to_vec(self).expect("a quote document always serializes");
        hex_sha256(&bytes)
    }
}

pub fn hex_sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The largest discount the quote grants, in percent: line discounts and percentage coupons.
pub fn max_discount_percent(quote: &DetailedQuote, coupons: &[Coupon]) -> Option<Decimal> {
    quote
        .components
        .iter()
        .filter_map(|c| c.discount_percent)
        .chain(quote.add_ons.iter().filter_map(|a| a.discount_percent))
        .chain(coupons.iter().filter_map(|c| match &c.discount {
            CouponDiscount::Percentage(p) => Some(*p),
            _ => None,
        }))
        .max()
}

/// The fee with every price taken down by `percent`, rounded half away from zero. Flat amounts
/// (packages, tier flat fees and caps) round to the currency's precision; unit rates, tier rates
/// included, keep two more decimals than they had.
pub fn discount_fee(fee: &SubscriptionFee, percent: Decimal, precision: u32) -> SubscriptionFee {
    let factor = (Decimal::ONE_HUNDRED - percent) / Decimal::ONE_HUNDRED;
    let half_up = |d: Decimal, dp: u32| {
        d.round_dp_with_strategy(dp, rust_decimal::RoundingStrategy::MidpointAwayFromZero)
            .normalize()
    };
    let flat = |d: &Decimal| half_up(d * factor, precision);
    let unit = |d: &Decimal| half_up(d * factor, d.scale().max(precision) + 2);

    let discount_model = |model: &mut RatingModel| match model {
        RatingModel::PerUnit { rate } => *rate = unit(rate),
        RatingModel::Package { rate, .. } => *rate = flat(rate),
        RatingModel::Tiered { tiers } | RatingModel::Volume { tiers } => {
            for tier in tiers {
                tier.rate = unit(&tier.rate);
                tier.flat_fee = tier.flat_fee.as_ref().map(flat);
                tier.flat_cap = tier.flat_cap.as_ref().map(flat);
            }
        }
    };

    let mut fee = fee.clone();
    match &mut fee {
        SubscriptionFee::OneTime { rate, model, .. }
        | SubscriptionFee::Recurring { rate, model, .. } => match model {
            // The rate only heads a modelled fee: it follows the discounted model.
            Some(model) => {
                discount_model(model);
                *rate = model.headline_rate();
            }
            None => *rate = flat(rate),
        },
        SubscriptionFee::Rate { rate } | SubscriptionFee::Credit { rate, .. } => *rate = flat(rate),
        SubscriptionFee::Capacity {
            rate, overage_rate, ..
        } => {
            *rate = flat(rate);
            *overage_rate = unit(overage_rate);
        }
        SubscriptionFee::Slot { model, .. } => discount_model(model),
        SubscriptionFee::Usage { model, .. } => match model {
            UsagePricingModel::PerUnit { rate } => *rate = unit(rate),
            UsagePricingModel::Package { rate, .. } => *rate = flat(rate),
            UsagePricingModel::Tiered { tiers, .. } | UsagePricingModel::Volume { tiers, .. } => {
                for tier in tiers {
                    tier.rate = unit(&tier.rate);
                    tier.flat_fee = tier.flat_fee.as_ref().map(flat);
                    tier.flat_cap = tier.flat_cap.as_ref().map(flat);
                }
            }
            UsagePricingModel::DimensionRates(rates) => {
                for rate in &mut rates.rates {
                    rate.rate = unit(&rate.rate);
                }
            }
            UsagePricingModel::RateCard { multiplier, .. } => {
                *multiplier = Some(unit(&multiplier.unwrap_or(Decimal::ONE)));
            }
        },
    }
    fee
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn document(coupons: Vec<QuoteDocumentCoupon>) -> QuoteDocument {
        let party = || QuoteDocumentParty {
            name: "Acme".to_string(),
            address: None,
            vat_number: None,
        };
        QuoteDocument {
            quote_number: "Q-1".to_string(),
            revision: 1,
            currency: "EUR".to_string(),
            seller: party(),
            customer: party(),
            billing_start_date: None,
            billing_end_date: None,
            trial_duration_days: None,
            billing_day_anchor: None,
            expires_at: None,
            net_terms: 30,
            overview: None,
            terms_and_services: None,
            sections: vec![],
            display_options: QuoteDisplayOptions::default(),
            lines: vec![],
            coupon_ids: vec![CouponId::default()],
            coupons,
            minimum: None,
            recipients: vec![],
            countersigners: vec![],
            attachments: vec![],
            consent_text: None,
        }
    }

    #[test]
    fn a_document_recorded_before_coupon_terms_still_reads() {
        let old = serde_json::to_value(document(vec![])).unwrap();
        assert!(old.get("coupons").is_none(), "no terms key without coupons");
        assert_eq!(old["coupon_ids"].as_array().map(Vec::len), Some(1));

        let read: QuoteDocument = serde_json::from_value(old).unwrap();
        assert_eq!(read.coupon_ids.len(), 1);
        assert!(read.coupons.is_empty());
    }

    #[test]
    fn coupon_terms_are_part_of_the_hash() {
        let terms = QuoteDocumentCoupon {
            code: "WELCOME20".to_string(),
            description: None,
            discount: CouponDiscount::Percentage(dec!(20)),
            recurring_value: Some(3),
            duration_months: None,
        };
        let with_terms = document(vec![terms.clone()]);
        assert_ne!(with_terms.hash(), document(vec![]).hash());

        let read: QuoteDocument =
            serde_json::from_value(serde_json::to_value(&with_terms).unwrap()).unwrap();
        assert_eq!(read.coupons, vec![terms]);
        assert_eq!(read.hash(), with_terms.hash());
    }

    #[test]
    fn an_attachment_content_is_part_of_the_hash() {
        let without = document(vec![]);
        assert_eq!(
            serde_json::to_value(&without).unwrap()["attachments"],
            serde_json::json!([])
        );

        let file = |sha256: &str| QuoteAttachment {
            id: common_domain::ids::QuoteAttachmentId::default(),
            filename: "MSA.pdf".to_string(),
            content_type: "application/pdf".to_string(),
            size_bytes: 1024,
            sha256: sha256.to_string(),
        };
        let mut with_msa = without.clone();
        with_msa.attachments = vec![file("aa")];
        let mut with_other_msa = with_msa.clone();
        with_other_msa.attachments = vec![file("bb")];
        assert_ne!(with_msa.hash(), without.hash());
        assert_ne!(with_msa.hash(), with_other_msa.hash());
    }

    #[test]
    fn a_coupons_months_are_part_of_its_terms_and_absent_before() {
        let counted = QuoteDocumentCoupon {
            code: "YC26".to_string(),
            description: None,
            discount: CouponDiscount::Percentage(dec!(50)),
            recurring_value: None,
            duration_months: None,
        };
        let json = serde_json::to_value(&counted).unwrap();
        assert!(
            json.get("duration_months").is_none(),
            "an older document's hash holds"
        );

        let year = QuoteDocumentCoupon {
            duration_months: Some(12),
            ..counted.clone()
        };
        assert_ne!(
            document(vec![year.clone()]).hash(),
            document(vec![counted]).hash()
        );
        let read: QuoteDocumentCoupon =
            serde_json::from_value(serde_json::to_value(&year).unwrap()).unwrap();
        assert_eq!(read, year);
    }

    #[test]
    fn a_flat_fee_rounds_to_the_currency() {
        let fee = discount_fee(&SubscriptionFee::Rate { rate: dec!(99.99) }, dec!(12.5), 2);
        assert!(matches!(fee, SubscriptionFee::Rate { rate } if rate == dec!(87.49)));
    }

    #[test]
    fn a_half_cent_rounds_away_from_zero() {
        let fee = discount_fee(&SubscriptionFee::Rate { rate: dec!(10.05) }, dec!(50), 2);
        assert!(matches!(fee, SubscriptionFee::Rate { rate } if rate == dec!(5.03)));
    }

    #[test]
    fn a_unit_rate_keeps_its_precision() {
        let fee = discount_fee(
            &SubscriptionFee::Usage {
                metric_id: common_domain::ids::BillableMetricId::default(),
                model: UsagePricingModel::PerUnit { rate: dec!(0.0015) },
                credit_unit: None,
            },
            dec!(10),
            2,
        );
        match fee {
            SubscriptionFee::Usage {
                model: UsagePricingModel::PerUnit { rate },
                ..
            } => assert_eq!(rate, dec!(0.00135)),
            _ => panic!("still a per-unit usage fee"),
        }
    }

    #[test]
    fn a_tier_rate_keeps_its_precision() {
        let tier = |first_unit: u64, rate: Decimal| crate::domain::price_components::TierRow {
            first_unit,
            rate,
            flat_fee: None,
            flat_cap: None,
        };
        let fee = discount_fee(
            &SubscriptionFee::Recurring {
                rate: dec!(0.045),
                quantity: 1000,
                billing_type: crate::domain::enums::BillingType::Advance,
                model: Some(RatingModel::Tiered {
                    tiers: vec![tier(0, dec!(0.045)), tier(500, dec!(0.03))],
                }),
            },
            dec!(10),
            2,
        );
        match fee {
            SubscriptionFee::Recurring {
                rate,
                model: Some(RatingModel::Tiered { tiers }),
                ..
            } => {
                assert_eq!(tiers[0].rate, dec!(0.0405));
                assert_eq!(tiers[1].rate, dec!(0.027));
                assert_eq!(rate, dec!(0.0405));
            }
            _ => panic!("still a tiered recurring fee"),
        }
    }

    #[test]
    fn a_package_rate_rounds_to_the_currency() {
        let fee = discount_fee(
            &SubscriptionFee::OneTime {
                rate: dec!(30),
                quantity: 25,
                model: Some(RatingModel::Package {
                    block_size: 10,
                    rate: dec!(30.05),
                }),
            },
            dec!(50),
            2,
        );
        assert!(matches!(
            fee,
            SubscriptionFee::OneTime { model: Some(RatingModel::Package { rate, .. }), .. }
                if rate == dec!(15.03)
        ));
    }

    #[test]
    fn a_full_discount_makes_the_line_free() {
        let fee = discount_fee(
            &SubscriptionFee::Slot {
                unit: "seat".into(),
                quantity_id: None,
                model: crate::domain::price_components::RatingModel::PerUnit { rate: dec!(25) },
                min_slots: None,
                max_slots: None,
                initial_slots: 3,
            },
            dec!(100),
            2,
        );
        assert!(matches!(fee, SubscriptionFee::Slot { model, .. } if !model.charges()));
    }
}
