mod layer;

pub use layer::MetricLayer;
pub use layer::MetricService;

pub fn create() -> MetricLayer {
    MetricLayer::new()
}
