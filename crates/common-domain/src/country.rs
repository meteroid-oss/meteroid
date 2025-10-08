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
        rust_iso3166::from_alpha2(code).map(|s| s.into())
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
        Self::parse_as_opt(s)
            .ok_or_else(|| CountryCodeError(format!("Invalid country code: {}", s)))
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
