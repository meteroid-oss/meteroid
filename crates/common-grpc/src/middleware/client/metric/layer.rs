use std::future::Future;
use std::pin::Pin;
use std::time::Instant;
use std::{
    error::Error,
    task::{Context, Poll},
};

use futures::ready;
use hyper::Request;
use hyper::Response;
use pin_project::pin_project;
use tonic::client::GrpcService;
use tonic::{Code, Status};
use tower::Layer;

use crate::{GrpcKind, GrpcServiceMethod};

#[derive(Debug, Default, Clone)]
pub struct MetricLayer {}

impl MetricLayer {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        MetricLayer {}
    }
}

impl<S> Layer<S> for MetricLayer {
    type Service = MetricService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        MetricService { inner }
    }
}

#[derive(Debug, Clone)]
pub struct MetricService<S> {
    inner: S,
}

impl<S, ReqBody, ResBody> GrpcService<ReqBody> for MetricService<S>
where
    S: GrpcService<ReqBody, ResponseBody = ResBody> + Clone + Send + 'static,
    S::Future: Send + 'static,
    S::Error: Error + 'static,
    ReqBody: Send + 'static,
    ResBody: http_body::Body + 'static,
{
    type ResponseBody = ResBody;
    type Error = S::Error;
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

        let started_at = std::time::Instant::now();
        let sm = GrpcServiceMethod::extract(request.uri());

        let future = inner.call(request);

        ResponseFuture {
            future,
            started_at,
            sm,
        }
    }
}

#[pin_project]
pub struct ResponseFuture<F> {
    #[pin]
    future: F,
    started_at: Instant,
    sm: GrpcServiceMethod,
}

impl<Fut, ResBody, E> Future for ResponseFuture<Fut>
where
    Fut: Future<Output = Result<Response<ResBody>, E>>,
    E: Error + 'static,
{
    type Output = Result<Response<ResBody>, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let res = ready!(this.future.poll(cx));

        let finished_at = Instant::now();
        let delta = finished_at.duration_since(*this.started_at).as_millis();

        let (res, grpc_status_code) = match res {
            Ok(result) => {
                let code = Status::from_header_map(result.headers())
                    .map(|s| s.code())
                    .unwrap_or(Code::Ok);
                (Ok(result), code)
            }
            Err(err) => {
                // tonic::transport::Error
                (Err::<_, E>(err), Code::Unavailable)
            }
        };

        super::super::super::common::metric::record_call(
            GrpcKind::CLIENT,
            this.sm.clone(),
            grpc_status_code,
            delta as u64,
        );

        Poll::Ready(res)
    }
}
