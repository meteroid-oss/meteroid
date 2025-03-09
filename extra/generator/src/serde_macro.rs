// MIT License, taken from https://github.com/Roger/serde-with-expand-env/blob/master/src/lib.rs
use serde::{Deserialize, Deserializer, de::Error};
use std::fmt::Display;
use std::str::FromStr;

// Allows resolving environment variables during deserialization.
pub fn with_expand_envs<'de, T, D>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: FromStr + serde::Deserialize<'de>,
    <T as FromStr>::Err: Display,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrAnything<T> {
        String(String),
        Anything(T),
    }

    match StringOrAnything::<T>::deserialize(deserializer)? {
        StringOrAnything::String(s) => match shellexpand::env(&s) {
            Ok(value) => value.parse::<T>().map_err(Error::custom),
            Err(err) => Err(Error::custom(err)),
        },
        StringOrAnything::Anything(anything) => Ok(anything),
    }
}
