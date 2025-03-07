use crate::id_type;
use sealed::sealed;
use std::error::Error;
use std::fmt::{Debug, Display};
use std::ops::Deref;
use std::str::FromStr;
use uuid::Uuid;

mod alias_or;
mod macros;

pub use alias_or::AliasOr;

id_type!(OrganizationId, "org_");
id_type!(TenantId, "ten_");
id_type!(CustomerId, "cus_");
id_type!(SubscriptionId, "sub_");
id_type!(InvoiceId, "inv_");
id_type!(InvoicingEntityId, "ive_");
id_type!(AddOnId, "add_");
id_type!(BankAccountId, "ba_");
id_type!(BillableMetricId, "bm_");
id_type!(CouponId, "cou_");
id_type!(CreditNoteId, "crn_");
id_type!(CustomerPaymentMethodId, "pm_");
id_type!(CustomerConnectionId, "ctn_");
id_type!(ConnectorId, "ctr_");
id_type!(EventId, "evt_");
id_type!(PaymentTransactionId, "pay_");
id_type!(ProductFamilyId, "pf_");
id_type!(ProductId, "prd_");
id_type!(PriceComponentId, "price_");
id_type!(PlanId, "plan_");

#[derive(Debug)]
pub struct IdError(pub(crate) String);
impl Display for IdError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "IdError: {}", self.0)
    }
}

impl Error for IdError {}

#[sealed]
pub trait BaseId: Deref<Target = Uuid> {
    const PREFIX: &'static str;
    type IdType;

    fn new() -> Self::IdType;
    fn as_uuid(&self) -> Uuid {
        **self
    }
    fn parse_uuid(s: &str) -> Result<Self::IdType, IdError>;

    fn as_base62(&self) -> String {
        format!(
            "{}{}",
            Self::PREFIX,
            base62::encode(self.as_uuid().as_u128())
        )
    }

    fn parse_base62(s: &str) -> Result<Self::IdType, IdError>;
}

pub mod string_serde {
    use crate::ids::{BaseId, IdError};
    use serde::{Deserialize, Deserializer, Serializer};
    use std::str::FromStr;

    pub fn serialize<S, T>(id: &T, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        T: BaseId + std::fmt::Display,
    {
        serializer.serialize_str(&id.to_string())
    }

    pub fn deserialize<'de, D, T>(deserializer: D) -> Result<T, D::Error>
    where
        D: Deserializer<'de>,
        T: BaseId + FromStr<Err = IdError> + std::fmt::Display,
    {
        let s = String::deserialize(deserializer)?;
        T::from_str(&s).map_err(serde::de::Error::custom)
    }
}

pub mod string_serde_opt {
    use crate::ids::BaseId;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S, T>(id: &Option<T>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        T: BaseId + Serialize,
    {
        match id {
            Some(id) => serializer.serialize_some(&id),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
    where
        D: Deserializer<'de>,
        T: BaseId + Deserialize<'de>,
    {
        Option::deserialize(deserializer)
    }
}

#[cfg(test)]
mod tests {
    use crate::ids::BaseId;
    use crate::ids::{string_serde, CustomerId};
    use serde::Deserialize;
    use serde::Serialize;
    use serde_json::json;
    use std::str::FromStr;

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct CustomerStrSerde {
        #[serde(with = "string_serde")]
        id: CustomerId,
    }

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct CustomerDefaultSerde {
        id: CustomerId,
    }

    #[test]
    fn test_to_from_string() {
        let id = CustomerId::new();
        let id_str = id.to_string();
        let parsed_id = CustomerId::from_str(&id_str).unwrap();
        assert_eq!(id, parsed_id)
    }

    #[test]
    fn test_default() {
        let id = CustomerId::default();
        let id2 = CustomerId::default();

        assert_eq!(id, id2);
        assert_eq!(id.to_string().as_str(), "cus_7n42DGM5Tflk9n8mt7Fhc7")
    }

    #[test]
    fn test_parse_uuid() {
        let id = CustomerId::new();
        let id_str = id.0.to_string();
        let parsed_id = CustomerId::parse_uuid(&id_str).unwrap();
        assert_eq!(id, parsed_id)
    }

    #[test]
    fn test_default_uuid_serde() {
        let cus = CustomerDefaultSerde {
            id: CustomerId::default(),
        };
        let actual_ser = serde_json::to_value(&cus).unwrap();
        let expected_ser = json!({"id": "ffffffff-ffff-ffff-ffff-ffffffffffff"});

        assert_eq!(actual_ser, expected_ser);

        let deserialized: CustomerDefaultSerde = serde_json::from_value(expected_ser).unwrap();
        assert_eq!(deserialized, cus);
    }

    #[test]
    fn test_string_serde() {
        let cus = CustomerStrSerde {
            id: CustomerId::default(),
        };
        let actual_ser = serde_json::to_value(&cus).unwrap();
        let expected_ser = json!({"id": "cus_7n42DGM5Tflk9n8mt7Fhc7"});

        assert_eq!(actual_ser, expected_ser);

        let deserialized: CustomerStrSerde = serde_json::from_value(expected_ser).unwrap();

        assert_eq!(deserialized, cus);
    }

    #[test]
    fn test_default_deserialize() {
        let str_ser = json!({"id": "cus_7n42DGM5Tflk9n8mt7Fhc7"});
        let default_ser = json!({"id": "ffffffff-ffff-ffff-ffff-ffffffffffff"});

        let str_deser: CustomerDefaultSerde = serde_json::from_value(str_ser).unwrap();
        let default_deser: CustomerDefaultSerde = serde_json::from_value(default_ser).unwrap();

        assert_eq!(str_deser, default_deser);
        assert_eq!(str_deser.id, CustomerId::default());
    }
}
