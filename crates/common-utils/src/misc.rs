use std::convert::Infallible;

pub trait UnwrapInfallible {
    type Ok;
    fn unwrap_infallible(self) -> Self::Ok;
}
impl<T> UnwrapInfallible for Result<T, Infallible> {
    type Ok = T;
    fn unwrap_infallible(self) -> T {
        match self {
            Ok(val) => val,
            Err(never) => match never {},
        }
    }
}
