use crate::domain::enums::CustomPropertyType;

/// A predicate operator. The subset valid for a given field is constrained by the field's
/// type via [`FilterOperator::allowed_for`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterOperator {
    Equals,
    NotEquals,
    In,
    Contains,
    Gt,
    Gte,
    Lt,
    Lte,
    Between,
    IsSet,
    IsEmpty,
    IsTrue,
    IsFalse,
    Before,
    After,
}

impl FilterOperator {
    /// Operators allowed on a custom property of the given type. Filtering is intentionally more
    /// permissive than writing (e.g. no SELECT option-membership check) so legacy values stay filterable.
    pub fn allowed_for(property_type: CustomPropertyType) -> &'static [FilterOperator] {
        use FilterOperator::*;
        match property_type {
            CustomPropertyType::Text | CustomPropertyType::Url | CustomPropertyType::Email => {
                &[Equals, NotEquals, In, Contains, IsSet, IsEmpty]
            }
            CustomPropertyType::Number => &[
                Equals, NotEquals, In, Gt, Gte, Lt, Lte, Between, IsSet, IsEmpty,
            ],
            CustomPropertyType::Boolean => &[IsTrue, IsFalse, IsSet],
            CustomPropertyType::Date | CustomPropertyType::Datetime => {
                &[Equals, Before, After, Between, IsSet, IsEmpty]
            }
            CustomPropertyType::SingleSelect => &[Equals, NotEquals, In, IsSet, IsEmpty],
            CustomPropertyType::MultiSelect => &[Contains, In, IsSet, IsEmpty],
            CustomPropertyType::Json => &[IsSet, IsEmpty],
        }
    }

    /// Whether this operator reads any values (IS_SET/IS_EMPTY/IS_TRUE/IS_FALSE take none).
    pub fn takes_values(self) -> bool {
        !matches!(
            self,
            FilterOperator::IsSet
                | FilterOperator::IsEmpty
                | FilterOperator::IsTrue
                | FilterOperator::IsFalse
        )
    }
}

/// A filter field: either a native entity column, or a custom property (wire-encoded `cp:<key>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldRef {
    Native(String),
    CustomProperty(String),
}

impl FieldRef {
    pub fn parse(field: &str) -> Self {
        match field.strip_prefix("cp:") {
            Some(key) => FieldRef::CustomProperty(key.to_string()),
            None => FieldRef::Native(field.to_string()),
        }
    }
}

/// A single filter as received from the API: values are raw strings, coerced later against the
/// field's type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filter {
    pub field: FieldRef,
    pub op: FilterOperator,
    pub values: Vec<String>,
}
