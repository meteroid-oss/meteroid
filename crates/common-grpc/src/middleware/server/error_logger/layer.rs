use futures::ready;
use http::{Request, Response};
use pin_project::pin_project;
use std::marker::PhantomData;
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};
use tonic::metadata::MetadataMap;
use tonic::{Code, Status};
use tower::{BoxError, Layer, Service};
use tracing::log::{Level, MetadataBuilder, Record, logger};

use common_grpc_error_as_tonic_macros::{HEADER_SOURCE_DETAILS, SourceDetails};

use crate::GrpcServiceMethod;

fn level_for_code(code: Code) -> Option<Level> {
    match code {
        Code::Ok => None,
        Code::Internal | Code::Unknown | Code::DataLoss | Code::Unavailable | Code::Aborted => {
            Some(Level::Error)
        }
        _ => Some(Level::Warn),
    }
}

#[derive(Debug, Clone, Default)]
pub struct ErrorLoggerLayer;

impl ErrorLoggerLayer {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        ErrorLoggerLayer {}
    }
}

impl<S> Layer<S> for ErrorLoggerLayer {
    type Service = ErrorLoggerService<S>;

    fn layer(&self, service: S) -> Self::Service {
        ErrorLoggerService { inner: service }
    }
}

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for ErrorLoggerService<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    S::Error: Into<BoxError>,
    ReqBody: Send,
{
    type Response = S::Response;
    // A concrete error, not `BoxError`: the OTel layer wrapping this one requires
    // `std::error::Error`.
    type Error = Status;
    type Future = ResponseFuture<S::Future, S::Error>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner
            .poll_ready(cx)
            .map_err(|err| Status::from_error(err.into()))
    }

    fn call(&mut self, request: Request<ReqBody>) -> Self::Future {
        // This is necessary because tonic internally uses `tower::buffer::Buffer`.
        // See https://github.com/tower-rs/tower/issues/547#issuecomment-767629149
        // for details on why this is necessary
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);

        let sm = GrpcServiceMethod::extract(request.uri());

        let future = inner.call(request);

        ResponseFuture {
            future,
            sm,
            _err: PhantomData,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ErrorLoggerService<S> {
    inner: S,
}

#[pin_project]
pub struct ResponseFuture<F, E> {
    #[pin]
    future: F,
    sm: GrpcServiceMethod,
    _err: PhantomData<E>,
}

impl<F, E, ResBody> Future for ResponseFuture<F, E>
where
    F: Future<Output = Result<Response<ResBody>, E>>,
    E: Into<BoxError>,
{
    type Output = Result<Response<ResBody>, Status>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let future_result = ready!(this.future.poll(cx));

        let result = match future_result {
            Ok(mut response) => {
                let maybe_status = Status::from_header_map(response.headers());
                let metadata_map = MetadataMap::from_headers(response.headers().clone());

                // removing custom header from response
                // because this workaround is necessary for logging
                // source details error only on server side
                let _ = response.headers_mut().remove(HEADER_SOURCE_DETAILS);

                if let Some(status) = maybe_status {
                    if let Some(level) = level_for_code(status.code()) {
                        let maybe_header_source_details =
                            metadata_map.get_bin(HEADER_SOURCE_DETAILS);

                        if let Some(header_source_details) = maybe_header_source_details {
                            let bytes = header_source_details.to_bytes().unwrap();
                            let source_details: SourceDetails =
                                serde_json::from_slice(&bytes).unwrap();

                            logger().log(
                                &Record::builder()
                                    .metadata(
                                        MetadataBuilder::new()
                                            .target(source_details.location_file.as_str())
                                            .level(level)
                                            .build(),
                                    )
                                    .args(format_args!(
                                        "Failed to process gRPC {}/{} with {:?} due to {} : {}",
                                        this.sm.service,
                                        this.sm.method,
                                        status.code(),
                                        source_details.msg,
                                        source_details.source
                                    ))
                                    .file(Some(source_details.location_file.as_str()))
                                    .line(Some(source_details.location_line))
                                    .build(),
                            );
                        } else {
                            logger().log(
                                &Record::builder()
                                    .metadata(
                                        MetadataBuilder::new()
                                            .target(module_path!())
                                            .level(level)
                                            .build(),
                                    )
                                    .args(format_args!(
                                        "Failed to process gRPC {}/{} with {:?} : {}",
                                        this.sm.service,
                                        this.sm.method,
                                        status.code(),
                                        status.message()
                                    ))
                                    .build(),
                            );
                        }
                    }
                } else {
                    // not gRPC request?
                    // ignoring
                }

                Ok(response)
            }
            Err(err) => Err(Status::from_error(err.into())),
        };

        Poll::Ready(result)
    }
}
