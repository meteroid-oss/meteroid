#[macro_export]
macro_rules! id_type {
    ($id_name:ident, $id_prefix:literal) => {
        #[derive(Debug, PartialEq, Eq, Clone, Hash, serde::Serialize, serde::Deserialize)]
        #[cfg_attr(feature = "diesel", derive(diesel_derive_newtype::DieselNewType))]
        #[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
        #[cfg_attr(feature = "utoipa", schema(value_type = String))]
        pub struct $id_name(uuid::Uuid);

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
        }

        impl From<uuid::Uuid> for $id_name {
            fn from(uuid: uuid::Uuid) -> Self {
                $id_name(uuid)
            }
        }

        impl std::fmt::Display for $id_name {
            fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                write!(f, "{}{}", $id_prefix, base62::encode(self.0.as_u128()))
            }
        }

        impl std::str::FromStr for $id_name {
            type Err = $crate::ids::IdError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                s.strip_prefix($id_prefix)
                    .ok_or_else(|| $crate::ids::IdError("Invalid prefix".to_string()))
                    .and_then(|s| {
                        base62::decode(s)
                            .map_err(|e| $crate::ids::IdError(e.to_string()))
                            .map(uuid::Uuid::from_u128)
                            .map($id_name)
                    })
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use crate::ids::string_serde;
    use crate::ids::BaseId;
    use serde::Deserialize;
    use serde::Serialize;
    use serde_json::json;
    use std::str::FromStr;

    id_type!(FakeId, "fake_");

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct Fake {
        #[serde(with = "string_serde")]
        id: FakeId,
    }

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct Fake2 {
        id: FakeId,
    }

    #[test]
    fn test_to_from_string() {
        let id = FakeId::new();
        let id_str = id.to_string();
        let parsed_id = FakeId::from_str(&id_str).unwrap();
        assert_eq!(id, parsed_id)
    }

    #[test]
    fn test_default() {
        let id = FakeId::default();
        let id2 = FakeId::default();

        assert_eq!(id, id2);
        assert_eq!(id.to_string().as_str(), "fake_7n42DGM5Tflk9n8mt7Fhc7")
    }

    #[test]
    fn test_parse_uuid() {
        let id = FakeId::new();
        let id_str = id.0.to_string();
        let parsed_id = FakeId::parse_uuid(&id_str).unwrap();
        assert_eq!(id, parsed_id)
    }

    #[test]
    fn test_uuid_serde() {
        let fake = Fake2 {
            id: FakeId::default(),
        };
        let actual_ser = serde_json::to_value(&fake).unwrap();
        let expected_ser = json!({"id": "ffffffff-ffff-ffff-ffff-ffffffffffff"});

        assert_eq!(actual_ser, expected_ser);

        let deserialized: Fake2 = serde_json::from_value(expected_ser).unwrap();
        assert_eq!(deserialized, fake);
    }

    #[test]
    fn test_string_serde() {
        let fake = Fake {
            id: FakeId::default(),
        };
        let actual_ser = serde_json::to_value(&fake).unwrap();
        let expected_ser = json!({"id": "fake_7n42DGM5Tflk9n8mt7Fhc7"});

        assert_eq!(actual_ser, expected_ser);

        let deserialized: Fake = serde_json::from_value(expected_ser).unwrap();
        assert_eq!(deserialized, fake);
    }
}
