pub use layer::MetricLayer;
pub use layer::MetricService;

mod layer;

pub fn create() -> MetricLayer {
    MetricLayer::new()
}
