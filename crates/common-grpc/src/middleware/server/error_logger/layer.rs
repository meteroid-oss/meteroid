use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use futures::ready;
use http::{Request, Response};
use pin_project::pin_project;
use tonic::metadata::MetadataMap;
use tonic::{Code, Status};
use tower::{Layer, Service};
use tracing::log::{logger, Level, MetadataBuilder, Record};

use common_grpc_error_as_tonic_macros::{SourceDetails, HEADER_SOURCE_DETAILS};

use crate::GrpcServiceMethod;

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

type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for ErrorLoggerService<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>, Error = BoxError>
        + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
    ReqBody: Send,
{
    type Response = S::Response;
    type Error = BoxError;
    type Future = ResponseFuture<S::Future>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: Request<ReqBody>) -> Self::Future {
        // This is necessary because tonic internally uses `tower::buffer::Buffer`.
        // See https://github.com/tower-rs/tower/issues/547#issuecomment-767629149
        // for details on why this is necessary
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);

        let sm = GrpcServiceMethod::extract(request.uri());

        let future = inner.call(request);

        ResponseFuture { future, sm }
    }
}

#[derive(Debug, Clone)]
pub struct ErrorLoggerService<S> {
    inner: S,
}

#[pin_project]
pub struct ResponseFuture<F> {
    #[pin]
    future: F,
    sm: GrpcServiceMethod,
}

impl<F, ResBody> Future for ResponseFuture<F>
where
    F: Future<Output = Result<Response<ResBody>, BoxError>>,
{
    type Output = Result<Response<ResBody>, BoxError>;

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

                match maybe_status {
                    Some(status) => {
                        if status.code() != Code::Ok {
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
                                                .level(Level::Error)
                                                .build(),
                                        )
                                        .args(format_args!(
                                            "Failed to process gRPC {}/{} due to {} : {}",
                                            this.sm.service,
                                            this.sm.method,
                                            source_details.msg,
                                            source_details.source
                                        ))
                                        .file(Some(source_details.location_file.as_str()))
                                        .line(Some(source_details.location_line))
                                        .build(),
                                )
                            }
                        }
                    }
                    None => {
                        // not gRPC request?
                        // ignoring
                    }
                }

                Ok(response)
            }
            Err(err) => {
                let status = Status::from_error(err);
                Err::<_, BoxError>(status.clone().into())
            }
        };

        Poll::Ready(result)
    }
}
