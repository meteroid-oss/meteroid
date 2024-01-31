use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use pin_project::pin_project;

#[pin_project]
pub struct Timed<Fut, F>
where
    Fut: Future,
    F: FnMut(&Fut::Output, Duration),
{
    #[pin]
    inner: Fut,
    f: F,
    started_at: Option<Instant>,
}

impl<Fut, F> Future for Timed<Fut, F>
where
    Fut: Future,
    F: FnMut(&Fut::Output, Duration),
{
    type Output = Fut::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        let this = self.project();
        let started_at = this.started_at.get_or_insert_with(Instant::now);

        match this.inner.poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(v) => {
                let elapsed = started_at.elapsed();

                (this.f)(&v, elapsed);

                Poll::Ready(v)
            }
        }
    }
}

pub trait TimedExt: Sized + Future {
    fn timed<F>(self, f: F) -> Timed<Self, F>
    where
        F: FnMut(&Self::Output, Duration),
    {
        Timed {
            inner: self,
            f,
            started_at: None,
        }
    }
}

impl<F: Future> TimedExt for F {}
