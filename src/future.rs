use std::task::{Context, Poll, Waker};
use std::future::Future;
use std::marker::PhantomPinned;
use std::mem::ManuallyDrop;
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
pub struct FutResponse<'a, T>(
    // actually_send passes a reference to this, we want to keep it alive until that reference is set and this is polled again.
    ManuallyDrop<_FutResponse<'a, T>>
);


#[must_use = "the response won't actually send until you await or poll"]
#[repr(C)]
pub struct _FutResponse<'a, T> {
    response: Option<ProxyResponse>,
    held_future: Option<Box<dyn Future<Output=()> + 'a>>,
    message: Option<ProxyRequest>,
    proxy: &'a EventLoop,
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
        FutResponse(ManuallyDrop::new(_FutResponse {
            response: None,
            held_future: None,
            message: Some(message),
            proxy,
            convert,
            _p: PhantomPinned
        }))
    }

    fn finalize(&mut self, response: ProxyResponse) -> Poll<T> {
        let convert = self.0.convert;
        // SAFETY: Once we return we no longer need this
        unsafe { ManuallyDrop::drop(&mut self.0) };
        Poll::Ready(convert(response))
    }
}

impl<'a, T> Future for FutResponse<'a, T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // SAFETY
        let this_wrapper = unsafe { self.get_unchecked_mut() };
        let this = &mut *this_wrapper.0;
        if let Some(response) = this.response.take() {
            debug_assert!(this.message.is_none());
            this_wrapper.finalize(response)
        } else {
            if let Some(message) = this.message.take() {
                debug_assert!(this.held_future.is_none());
                this.held_future = Some(Box::new(this.proxy.actually_send(message, cx.waker().clone(), &mut this.response as *mut _)));
            } else {
                debug_assert!(this.held_future.is_some());
            }
            // SAFETY: in a Pin
            let held_future = unsafe { Pin::new_unchecked(this.held_future.as_mut().unwrap().as_mut()) };
            match held_future.poll(cx) {
                Poll::Ready(()) => {
                    let response = this.response.take().expect("FutResponse poll instantly succeeded but there is no response");
                    this_wrapper.finalize(response)
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