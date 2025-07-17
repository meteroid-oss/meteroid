pub mod conversions {
    use crate::errors::RestApiError;
    use std::str::FromStr;

    pub trait RestConv<T> {
        fn as_rest(&self) -> T;
        fn from_rest(rest: T) -> Result<Self, RestApiError>
        where
            Self: Sized,
        {
            Self::from_rest_ref(&rest)
        }
        fn from_rest_ref(rest: &T) -> Result<Self, RestApiError>
        where
            Self: Sized;
    }

    pub trait AsRestOpt<T> {
        #[allow(dead_code)]
        fn as_rest(&self) -> Option<T>
        where
            Self: Sized;
    }

    pub trait FromRestOpt<T>: RestConv<T> {
        fn from_rest_opt(rest: Option<T>) -> Result<Option<Self>, RestApiError>
        where
            Self: Sized;
    }

    impl<T, U> AsRestOpt<T> for Option<U>
    where
        U: RestConv<T>,
    {
        fn as_rest(&self) -> Option<T> {
            self.as_ref().map(|d| d.as_rest())
        }
    }

    impl<T, U> FromRestOpt<T> for U
    where
        U: RestConv<T>,
    {
        fn from_rest_opt(rest: Option<T>) -> Result<Option<Self>, RestApiError> {
            rest.map(U::from_rest).transpose()
        }
    }

    impl RestConv<String> for rust_decimal::Decimal {
        fn as_rest(&self) -> String {
            self.to_string()
        }

        fn from_rest_ref(rest: &String) -> Result<Self, RestApiError> {
            rust_decimal::Decimal::from_str(rest).map_err(|_| RestApiError::InvalidInput)
        }
    }
}
