use anyhow::anyhow;
use meteroid_grpc::meteroid::api::adjustments::v1::discount;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::str::FromStr;

pub trait Adjustment {
    fn apply(&self, base: Decimal) -> anyhow::Result<Decimal>;
}

impl Adjustment for discount::Percent {
    fn apply(&self, base: Decimal) -> anyhow::Result<Decimal> {
        match &self.percentage {
            Some(p) => {
                let dec_p: Decimal = Decimal::from_str(&p.value)
                    .map_err(|e| anyhow!("Failed to convert string to Decimal: {}", e))?;
                let discount_amount = base * dec_p / dec!(100.0);
                Ok(base - discount_amount)
            }
            None => Ok(base),
        }
    }
}
