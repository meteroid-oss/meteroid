use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
    time::Instant,
};

use futures::ready;
use http::{Request, Response};
use pin_project::pin_project;
use tonic::{Code, Status};
use tower::{BoxError, Layer, Service};

use crate::{GrpcKind, GrpcServiceMethod};

#[derive(Debug, Clone, Default)]
pub struct MetricLayer;

impl MetricLayer {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        MetricLayer {}
    }
}

impl<S> Layer<S> for MetricLayer {
    type Service = MetricService<S>;

    fn layer(&self, service: S) -> Self::Service {
        MetricService { inner: service }
    }
}

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for MetricService<S>
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

#[derive(Debug, Clone)]
pub struct MetricService<S> {
    inner: S,
}

#[pin_project]
pub struct ResponseFuture<F> {
    #[pin]
    future: F,
    started_at: Instant,
    sm: GrpcServiceMethod,
}

impl<F, ResBody> Future for ResponseFuture<F>
where
    F: Future<Output = Result<Response<ResBody>, BoxError>>,
{
    type Output = Result<Response<ResBody>, BoxError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let res = ready!(this.future.poll(cx));

        let finished_at = Instant::now();
        let delta = finished_at.duration_since(*this.started_at).as_millis();

        let (res, grpc_status_code) = match res {
            Ok(res) => (Ok(res), Code::Ok),
            Err(err) => {
                let status = Status::from_error(err);
                (Err::<_, BoxError>(status.clone().into()), status.code())
            }
        };

        super::super::super::common::metric::record_call(
            GrpcKind::SERVER,
            this.sm.clone(),
            grpc_status_code,
            delta as u64,
        );

        Poll::Ready(res)
    }
}
