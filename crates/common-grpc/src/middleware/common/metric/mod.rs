use crate::code::code_as_str;
use crate::{GrpcKind, GrpcServiceMethod};
use tonic::Code;

pub mod metrics;

pub fn record_call(kind: GrpcKind, sm: GrpcServiceMethod, status_code: Code, latency: u64) {
    let status_code_str = code_as_str(status_code);

    let attributes = &[
        metrics::KeyValue::new("grpc_kind", kind.to_string()),
        metrics::KeyValue::new("grpc_service", sm.service),
        metrics::KeyValue::new("grpc_method", sm.method),
        metrics::KeyValue::new("grpc_status", status_code_str),
    ];

    metrics::CALL_COUNTER.add(1, attributes);
    metrics::CALL_LATENCY.record(latency, attributes);
}
