//! Which customers may get a plan or redeem a coupon: by billing country and custom properties.
//! Live, never versioned, never a price.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::domain::customers::Customer;
use crate::json_value_serde;

/// Every condition set must hold; the default rule lets anyone in. Keys of older rules are
/// ignored when read.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Eligibility {
    /// ISO 3166-1 alpha-2 of the customer's billing address; empty: any country.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub countries: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub properties: Vec<PropertyMatch>,
}

json_value_serde!(Eligibility);

/// The customer's custom property `key` holds one of `values`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PropertyMatch {
    pub key: String,
    pub values: Vec<String>,
}

/// What a rule reads of a customer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CustomerFacts {
    /// ISO 3166-1 alpha-2 of the billing address.
    pub country: Option<String>,
    /// Each custom property's values, as text; a list property holds several.
    pub properties: BTreeMap<String, Vec<String>>,
}

impl CustomerFacts {
    /// `custom_properties` as a customer holds them: a JSON object keyed by property.
    pub fn new(country: Option<String>, custom_properties: &serde_json::Value) -> Self {
        CustomerFacts {
            country,
            properties: property_values(custom_properties),
        }
    }

    pub fn of(customer: &Customer) -> Self {
        let country = customer
            .billing_address
            .as_ref()
            .and_then(|address| address.country.as_ref())
            .map(|country| country.code.clone());
        Self::new(country, &customer.custom_properties)
    }
}

fn property_values(properties: &serde_json::Value) -> BTreeMap<String, Vec<String>> {
    fn text(value: &serde_json::Value) -> Option<String> {
        match value {
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Number(n) => Some(n.to_string()),
            serde_json::Value::Bool(b) => Some(b.to_string()),
            _ => None,
        }
    }
    let Some(object) = properties.as_object() else {
        return BTreeMap::new();
    };
    object
        .iter()
        .map(|(key, value)| {
            let values = match value {
                serde_json::Value::Array(items) => items.iter().filter_map(text).collect(),
                other => text(other).into_iter().collect(),
            };
            (key.clone(), values)
        })
        .collect()
}

/// Why a customer may not get something; `message` is shown to them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ineligible {
    /// Their billing address is in none of these countries, or they have none.
    Country(Vec<String>),
    /// A custom property of theirs holds none of the values asked.
    Property(String),
}

impl Ineligible {
    pub fn message(&self) -> String {
        match self {
            Ineligible::Country(countries) => {
                format!("This offer is for customers in {}", countries.join(", "))
            }
            Ineligible::Property(_) => "Your account does not qualify for this offer".to_owned(),
        }
    }
}

impl std::fmt::Display for Ineligible {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

/// Whether a customer with `facts` may get what `rule` guards.
pub fn check(rule: &Eligibility, facts: &CustomerFacts) -> Result<(), Ineligible> {
    if !rule.countries.is_empty() {
        let in_one = facts
            .country
            .as_deref()
            .map(str::trim)
            .is_some_and(|country| {
                rule.countries
                    .iter()
                    .any(|allowed| allowed.trim().eq_ignore_ascii_case(country))
            });
        if !in_one {
            return Err(Ineligible::Country(rule.countries.clone()));
        }
    }
    for wanted in &rule.properties {
        let holds = facts.properties.get(&wanted.key).is_some_and(|held| {
            held.iter().any(|value| {
                wanted
                    .values
                    .iter()
                    .any(|allowed| allowed.trim().eq_ignore_ascii_case(value.trim()))
            })
        });
        if !holds {
            return Err(Ineligible::Property(wanted.key.clone()));
        }
    }
    Ok(())
}

impl Eligibility {
    /// Lets anyone in: stored as no rule at all.
    pub fn is_open(&self) -> bool {
        *self == Self::default()
    }

    /// As stored: trimmed, countries upper-case, blanks and repeats dropped.
    pub fn normalized(self) -> Self {
        fn unique(values: impl IntoIterator<Item = String>) -> Vec<String> {
            let mut seen = Vec::new();
            for value in values {
                if !value.is_empty() && !seen.contains(&value) {
                    seen.push(value);
                }
            }
            seen
        }
        let mut properties: Vec<PropertyMatch> = Vec::new();
        for wanted in self.properties {
            let key = wanted.key.trim().to_owned();
            let values = unique(wanted.values.into_iter().map(|v| v.trim().to_owned()));
            match properties.iter_mut().find(|p| p.key == key) {
                Some(existing) => {
                    let held = std::mem::take(&mut existing.values);
                    existing.values = unique(held.into_iter().chain(values));
                }
                None => properties.push(PropertyMatch { key, values }),
            }
        }
        Eligibility {
            countries: unique(
                self.countries
                    .into_iter()
                    .map(|c| c.trim().to_ascii_uppercase()),
            ),
            properties,
        }
    }

    /// Refuses a rule nobody could read the same way twice; `normalized` first.
    pub fn validate(&self) -> Result<(), String> {
        if let Some(country) = self
            .countries
            .iter()
            .find(|c| common_domain::country::CountryCode::parse_as_opt(c).is_none())
        {
            return Err(format!(
                "{country} is not an ISO 3166-1 alpha-2 country code"
            ));
        }
        if let Some(wanted) = self
            .properties
            .iter()
            .find(|p| p.key.is_empty() || p.values.is_empty())
        {
            return Err(format!(
                "property match '{}' needs a key and at least one value",
                wanted.key
            ));
        }
        Ok(())
    }

    /// The rule a stored value names, `None` when open. One that no longer reads lets nobody
    /// in rather than everybody: no customer holds a property with no key.
    pub fn from_row(value: Option<serde_json::Value>) -> Option<Eligibility> {
        let value = value.filter(|v| !v.is_null())?;
        let rule = serde_json::from_value::<Eligibility>(value).unwrap_or_else(|_| Eligibility {
            countries: Vec::new(),
            properties: vec![PropertyMatch {
                key: String::new(),
                values: Vec::new(),
            }],
        });
        (!rule.is_open()).then_some(rule)
    }

    /// The stored value of `rule`: none when it lets anyone in.
    pub fn to_row(rule: Option<&Eligibility>) -> Option<serde_json::Value> {
        rule.filter(|rule| !rule.is_open())
            .and_then(|rule| serde_json::to_value(rule).ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(country: Option<&str>) -> CustomerFacts {
        CustomerFacts {
            country: country.map(str::to_owned),
            properties: BTreeMap::new(),
        }
    }

    #[test]
    fn countries_and_properties_are_who_the_customer_is() {
        let rule = Eligibility {
            countries: vec!["BR".to_owned(), "PT".to_owned()],
            properties: vec![PropertyMatch {
                key: "student".to_owned(),
                values: vec!["verified".to_owned()],
            }],
        };
        let mut student = facts(Some("br"));
        student
            .properties
            .insert("student".to_owned(), vec!["Verified".to_owned()]);
        assert!(check(&rule, &student).is_ok());
        assert_eq!(
            check(&rule, &facts(Some("US"))),
            Err(Ineligible::Country(vec!["BR".to_owned(), "PT".to_owned()]))
        );
        assert_eq!(
            check(&rule, &facts(Some("PT"))),
            Err(Ineligible::Property("student".to_owned()))
        );
        assert!(check(&rule, &facts(None)).is_err());
        assert!(check(&Eligibility::default(), &facts(None)).is_ok());
    }

    #[test]
    fn a_rule_is_stored_normalized_and_an_open_one_not_at_all() {
        let rule = Eligibility {
            countries: vec![" fr ".to_owned(), "FR".to_owned()],
            ..Default::default()
        }
        .normalized();
        assert_eq!(rule.countries, vec!["FR".to_owned()]);
        assert!(rule.validate().is_ok());
        assert_eq!(Eligibility::to_row(Some(&Eligibility::default())), None);
        let stored = Eligibility::to_row(Some(&rule));
        assert_eq!(Eligibility::from_row(stored), Some(rule));
        assert!(
            Eligibility {
                countries: vec!["XX".to_owned()],
                ..Default::default()
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn an_older_rule_still_reads_and_an_unreadable_one_lets_nobody_in() {
        let older = Eligibility::from_row(Some(serde_json::json!({
            "hidden": true,
            "countries": ["BR"],
            "email_domains": ["edu.br"],
            "invitation_code": "YC26",
            "approval_required": false,
        })));
        assert_eq!(
            older.map(|rule| rule.countries),
            Some(vec!["BR".to_owned()])
        );
        let unreadable =
            Eligibility::from_row(Some(serde_json::json!({ "countries": "BR" }))).expect("a rule");
        assert!(check(&unreadable, &facts(Some("BR"))).is_err());
    }
}
