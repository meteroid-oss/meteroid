use serde::{Deserialize, Serialize};

use crate::serde_macro::with_expand_envs;

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub connect: Connect,
    pub events_per_second: u32,
    pub limit: Option<u32>,
    pub events: Vec<Schema>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Connect {
    #[serde(deserialize_with = "with_expand_envs")]
    pub endpoint: String,
    pub api_key: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Schema {
    pub event_name: String,
    pub customer_aliases: Vec<String>,
    pub properties: std::collections::HashMap<String, Property>,
    pub weight: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Property {
    Typed(DataType),
    Fixed(FixedValue),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FixedValue {
    Boolean(bool),
    String(String),
    Float(f64),
    Integer(i64),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
#[allow(non_camel_case_types)]
pub enum DataType {
    int { min: Option<i32>, max: Option<i32> },
    float { min: Option<f64>, max: Option<f64> },
    bool,
    string { length: Option<usize> },
    pick { values: Vec<FixedValue> },
}
