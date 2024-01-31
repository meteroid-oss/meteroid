// todo use smth like https://github.com/greyblake/nutype

newtype_uuid!(TenantId);
newtype_uuid!(InvoiceId);
newtype_uuid!(InvoiceLineId);
newtype_uuid!(CustomerId);
newtype_uuid!(SubscriptionId);
newtype_uuid!(PlanId);
newtype_uuid!(PricePointId);

newtype_string_secret!(StripeSecret);
newtype_string_secret!(StripeWebhookSecret);

#[macro_export]
macro_rules! newtype_uuid {
    ($wrapper_name:ident) => {
        #[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Hash, serde::Serialize)]
        pub struct $wrapper_name(pub uuid::Uuid);

        impl std::fmt::Display for $wrapper_name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl std::str::FromStr for $wrapper_name {
            type Err = uuid::Error;

            fn from_str(str: &str) -> Result<Self, Self::Err> {
                uuid::Uuid::from_str(str).map(|x| $wrapper_name(x))
            }
        }

        impl From<uuid::Uuid> for $wrapper_name {
            fn from(uuid: uuid::Uuid) -> Self {
                $wrapper_name(uuid)
            }
        }

        impl<'de> serde::Deserialize<'de> for $wrapper_name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                <uuid::Uuid>::deserialize(deserializer).map(|x| $wrapper_name(x))
            }
        }
    };
}

#[macro_export]
macro_rules! newtype_string {
    ($wrapper_name:ident) => {
        #[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Hash, serde::Serialize)]
        pub struct $wrapper_name(pub String);

        impl std::fmt::Display for $wrapper_name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl std::str::FromStr for $wrapper_name {
            type Err = std::convert::Infallible;

            fn from_str(str: &str) -> Result<Self, Self::Err> {
                Ok($wrapper_name(str.to_string()))
            }
        }

        impl<'de> serde::Deserialize<'de> for $wrapper_name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                <String>::deserialize(deserializer).map(|x| $wrapper_name(x))
            }
        }
    };
}

#[macro_export]
macro_rules! newtype_string_secret {
    ($wrapper_name:ident) => {
        #[derive(Clone)]
        pub struct $wrapper_name(pub secrecy::SecretString);

        impl std::str::FromStr for $wrapper_name {
            type Err = std::convert::Infallible;

            fn from_str(str: &str) -> Result<Self, Self::Err> {
                secrecy::SecretString::from_str(str).map(|x| $wrapper_name(x))
            }
        }

        impl From<String> for $wrapper_name {
            fn from(string: String) -> Self {
                $wrapper_name(string.into())
            }
        }
    };
}
