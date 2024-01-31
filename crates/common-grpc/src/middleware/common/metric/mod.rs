use crate::code::code_as_str;
use crate::{GrpcKind, GrpcServiceMethod};
use tonic::Code;

pub mod metrics;

pub fn record_call(kind: GrpcKind, sm: GrpcServiceMethod, status_code: Code, latency: u64) {
    let status_code_str = code_as_str(status_code);

    let attributes = &[
        metrics::KeyValue {
            key: "grpc_kind".into(),
            value: kind.to_string().into(),
        },
        metrics::KeyValue {
            key: "grpc_service".into(),
            value: sm.service.into(),
        },
        metrics::KeyValue {
            key: "grpc_method".into(),
            value: sm.method.into(),
        },
        metrics::KeyValue {
            key: "grpc_status".into(),
            value: status_code_str.into(),
        },
    ];

    metrics::CALL_COUNTER.add(1, attributes);
    metrics::CALL_LATENCY.record(latency, attributes);
}
