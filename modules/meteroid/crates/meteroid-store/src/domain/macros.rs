#[macro_export]
macro_rules! json_value_serde {
    ($t:ty) => {
        impl TryFrom<serde_json::Value> for $t {
            type Error = error_stack::Report<$crate::errors::StoreError>;

            fn try_from(value: serde_json::Value) -> Result<Self, Self::Error> {
                serde_json::from_value(value).map_err(|e| {
                    error_stack::report!($crate::errors::StoreError::SerdeError(
                        format!("Failed to deserialize {}", stringify!($t)),
                        e
                    ))
                })
            }
        }

        impl TryFrom<&serde_json::Value> for $t {
            type Error = error_stack::Report<$crate::errors::StoreError>;

            fn try_from(value: &serde_json::Value) -> Result<Self, Self::Error> {
                <$t as serde::Deserialize>::deserialize(value).map_err(|e| {
                    error_stack::report!($crate::errors::StoreError::SerdeError(
                        format!("Failed to deserialize {}", stringify!($t)),
                        e
                    ))
                })
            }
        }

        impl TryInto<serde_json::Value> for $t {
            type Error = error_stack::Report<$crate::errors::StoreError>;

            fn try_into(self) -> Result<serde_json::Value, Self::Error> {
                serde_json::to_value(self).map_err(|e| {
                    error_stack::report!($crate::errors::StoreError::SerdeError(
                        format!("Failed to serialize {}", stringify!($t)),
                        e
                    ))
                })
            }
        }

        impl<'a> TryInto<serde_json::Value> for &'a $t {
            type Error = error_stack::Report<$crate::errors::StoreError>;

            fn try_into(self) -> Result<serde_json::Value, Self::Error> {
                serde_json::to_value(self).map_err(|e| {
                    error_stack::report!($crate::errors::StoreError::SerdeError(
                        format!("Failed to serialize &{}", stringify!($t)),
                        e
                    ))
                })
            }
        }
    };
}

#[macro_export]
macro_rules! json_value_ser {
    ($t:ty) => {
        impl TryInto<serde_json::Value> for $t {
            type Error = error_stack::Report<$crate::errors::StoreError>;

            fn try_into(self) -> Result<serde_json::Value, Self::Error> {
                serde_json::to_value(self).map_err(|e| {
                    error_stack::report!($crate::errors::StoreError::SerdeError(
                        format!("Failed to serialize {}", stringify!($t)),
                        e
                    ))
                })
            }
        }

        impl<'a> TryInto<serde_json::Value> for &'a $t {
            type Error = error_stack::Report<$crate::errors::StoreError>;

            fn try_into(self) -> Result<serde_json::Value, Self::Error> {
                serde_json::to_value(self).map_err(|e| {
                    error_stack::report!($crate::errors::StoreError::SerdeError(
                        format!("Failed to serialize &{}", stringify!($t)),
                        e
                    ))
                })
            }
        }
    };
}
