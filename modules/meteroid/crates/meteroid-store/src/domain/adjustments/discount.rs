use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Amount {
    pub value_in_cents: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Percent {
    pub percentage: Decimal,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Quantity {
    pub quantity_value: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum StandardDiscount {
    Amount(Amount),
    Percent(Percent),
}
