#[macro_export]
macro_rules! id_type {
    ($id_name:ident, $id_prefix:literal) => {
        #[derive(Debug, PartialEq, Eq, Clone, Hash, serde::Serialize, Copy)]
        #[cfg_attr(feature = "diesel", derive(diesel_derive_newtype::DieselNewType))]
        pub struct $id_name(uuid::Uuid);

        #[cfg(feature = "utoipa")]
        impl utoipa::PartialSchema for $id_name {
            fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::Schema> {
                utoipa::openapi::schema::Object::builder()
                    .schema_type(utoipa::openapi::schema::SchemaType::Type(
                        utoipa::openapi::Type::String,
                    ))
                    .format(Some(utoipa::openapi::schema::SchemaFormat::Custom(
                        "MeteroidId".to_string(),
                    )))
                    .examples([$id_name::default().as_base62()])
                    .into()
            }
        }

        #[cfg(feature = "utoipa")]
        impl utoipa::ToSchema for $id_name {
            fn name() -> std::borrow::Cow<'static, str> {
                std::borrow::Cow::Borrowed(stringify!($id_name))
            }
        }

        impl $id_name {
            #[cfg(feature = "tonic")]
            pub fn as_proto(&self) -> String {
                self.as_base62()
            }

            #[cfg(feature = "tonic")]
            pub fn from_proto<T: AsRef<str>>(value: T) -> Result<$id_name, tonic::Status> {
                $id_name::from_str(value.as_ref()).map_err(|_| {
                    tonic::Status::invalid_argument(format!(
                        "Invalid {}: {}",
                        stringify!($id_name),
                        value.as_ref()
                    ))
                })
            }

            #[cfg(feature = "tonic")]
            pub fn from_proto_opt<T: AsRef<str>>(
                value: Option<T>,
            ) -> Result<Option<$id_name>, tonic::Status> {
                value.map($id_name::from_proto).transpose()
            }

            pub const fn from_const(uuid: uuid::Uuid) -> Self {
                $id_name(uuid)
            }
        }

        impl<'de> serde::Deserialize<'de> for $id_name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let s = String::deserialize(deserializer)?;

                if s.starts_with($id_prefix) {
                    $id_name::parse_base62(&s).map_err(serde::de::Error::custom)
                } else {
                    $id_name::parse_uuid(&s).map_err(serde::de::Error::custom)
                }
            }
        }

        impl std::ops::Deref for $id_name {
            type Target = uuid::Uuid;

            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl Default for $id_name {
            #[inline]
            fn default() -> Self {
                uuid::Uuid::max().into()
            }
        }

        #[sealed::sealed]
        impl $crate::ids::BaseId for $id_name {
            const PREFIX: &'static str = $id_prefix;
            type IdType = $id_name;

            fn new() -> Self::IdType {
                $id_name(uuid::Uuid::now_v7())
            }

            fn parse_uuid(s: &str) -> Result<$id_name, $crate::ids::IdError> {
                uuid::Uuid::parse_str(s)
                    .map_err(|e| $crate::ids::IdError(e.to_string()))
                    .map(|x| $id_name(x))
            }

            fn parse_base62(s: &str) -> Result<$id_name, $crate::ids::IdError> {
                s.strip_prefix($id_prefix)
                    .ok_or_else(|| $crate::ids::IdError("Invalid prefix".to_string()))
                    .and_then(|s| {
                        base62::decode(s)
                            .map_err(|e| $crate::ids::IdError(e.to_string()))
                            .map(|decoded| decoded.rotate_right(67))
                            .map(uuid::Uuid::from_u128)
                            .map($id_name)
                    })
            }
        }

        impl From<uuid::Uuid> for $id_name {
            fn from(uuid: uuid::Uuid) -> Self {
                $id_name(uuid)
            }
        }

        impl std::fmt::Display for $id_name {
            fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                write!(f, "{}", self.as_base62())
            }
        }

        impl std::str::FromStr for $id_name {
            type Err = $crate::ids::IdError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                if s.starts_with($id_prefix) {
                    $id_name::parse_base62(s)
                } else {
                    $id_name::parse_uuid(s)
                }
            }
        }
    };
}
