pub use layer::ErrorLoggerLayer;
pub use layer::ErrorLoggerService;

mod layer;

pub fn create() -> ErrorLoggerLayer {
    ErrorLoggerLayer::new()
}
