use std::fmt::{Display, Formatter};
use std::task::{Context, Poll, Waker};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use crossbeam_utils::atomic::AtomicCell;
use crate::event_loop::EventLoop;
use crate::messages::{ProxyRegisterBody, ProxyResponse};

pub struct FutEventLoop {
    pub(crate) body: Arc<AtomicCell<ProxyRegisterBody>>
}

pub struct FutResponse<'a, T> {
    pub(crate) proxy: &'a EventLoop,
    pub(crate) id: ResponseId,
    pub(crate) convert: fn(ProxyResponse) -> T
}

impl Future for FutEventLoop {
    type Output = EventLoop;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.body.take() {
            ProxyRegisterBody::Init => {
                self.body.store(ProxyRegisterBody::Polled { waker: cx.waker().clone() });
                Poll::Pending
            }
            ProxyRegisterBody::Polled { waker: _ } => panic!("polled redundantly"),
            ProxyRegisterBody::Ready { info } => Poll::Ready(EventLoop::from(info))
        }
    }
}

impl<'a, T> Future for FutResponse<'a, T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        this.proxy.poll(this.id, cx).map(this.convert)
    }
}

pub(crate) struct PendingResponse {
    pub(crate) id: ResponseId,
    waker: Option<Waker>
}

pub(crate) struct ReadyResponse {
    pub(crate) id: ResponseId,
    pub(crate) response: ProxyResponse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ResponseId(pub(crate) usize);

impl PendingResponse {
    pub(crate) fn new(id: ResponseId) -> Self {
        PendingResponse {
            id,
            waker: None
        }
    }

    pub(crate) fn wake(mut self, response: ProxyResponse) -> ReadyResponse {
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
        ReadyResponse::new(self.id, response)
    }

    pub(crate) fn poll(&mut self, cx: &mut Context<'_>) {
        assert!(self.waker.is_none(), "redundantly polled");
        self.waker = Some(cx.waker().clone());
    }
}

impl ReadyResponse {
    fn new(id: ResponseId, response: ProxyResponse) -> Self {
        ReadyResponse {
            id,
            response
        }
    }
}

impl Display for ResponseId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{}", self.0)
    }
}