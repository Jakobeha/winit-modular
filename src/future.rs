use std::task::{Context, Poll, Waker};
use std::future::Future;
use std::marker::PhantomPinned;
use std::pin::Pin;
use std::sync::Arc;
use crossbeam_utils::atomic::AtomicCell;
use crate::event_loop::EventLoop;
use crate::messages::{ProxyRegisterBody, ProxyRequest, ProxyResponse};

pub struct FutEventLoop {
    pub(crate) body: Arc<AtomicCell<ProxyRegisterBody>>
}

#[must_use = "the response won't actually send until you await or poll"]
#[repr(C)]
pub struct FutResponse<'a, T> {
    response: Option<ProxyResponse>,
    proxy: &'a EventLoop,
    message: Option<ProxyRequest>,
    convert: fn(ProxyResponse) -> T,
    // This is pinned because there is a pointer to response in PendingRequest
    _p: PhantomPinned
}

pub(crate) struct PendingRequest {
    waker: Option<Waker>,
    response_ptr: *mut Option<ProxyResponse>
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

impl<'a, T> FutResponse<'a, T> {
    pub(crate) fn new(
        proxy: &'a EventLoop,
        message: ProxyRequest,
        convert: fn(ProxyResponse) -> T
    ) -> Self {
        FutResponse {
            response: None,
            proxy,
            message: Some(message),
            convert,
            _p: PhantomPinned
        }
    }
}

impl<'a, T> Future for FutResponse<'a, T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // SAFETY
        let this = unsafe { self.get_unchecked_mut() };
        if let Some(response) = this.response.take() {
            debug_assert!(this.message.is_none());
            Poll::Ready((this.convert)(response))
        } else {
            let message = this.message.take().expect("redundantly polled");
            match Pin::new(&mut Box::pin(this.proxy.actually_send(message, cx.waker().clone(), &mut this.response as *mut _))).poll(cx) {
                Poll::Ready(()) => {
                    let response = this.response.take().expect("FutResponse poll instantly succeeded but there is no response");
                    Poll::Ready((this.convert)(response))
                }
                Poll::Pending => Poll::Pending
            }
        }
    }
}

impl PendingRequest {
    pub(crate) fn new(waker: Waker, response_ptr: *mut Option<ProxyResponse>) -> Self {
        PendingRequest {
            waker: Some(waker),
            response_ptr
        }
    }

    pub(crate) fn resolve(mut self, response: ProxyResponse) {
        unsafe { *self.response_ptr = Some(response); }
        let waker = self.waker.take().expect("redundantly resolved");
        waker.wake();
    }
}