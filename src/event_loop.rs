use std::cmp::Ordering;
use std::sync::Arc;
use crossbeam_utils::atomic::AtomicCell;
use flume::{Receiver, Sender, TryRecvError, TrySendError};
use std::cell::{Cell, RefCell, RefMut};
use std::collections::VecDeque;
use winit::window::{Window, WindowBuilder};
use winit::error::OsError;
use pollster::block_on;
use std::task::{Context, Poll};
use std::time::Instant;
use crate::event::Event;
use crate::future::{ReadyResponse, FutResponse, PendingResponse, FutEventLoop, ResponseId};
use crate::messages::{ProxyRegister, ProxyRegisterBody, ProxyRegisterInfo, ProxyRequest, ProxyResponse, REGISTER_PROXY};

/// Similar API to [winit::EventLoop], however
///
/// - You can create multiples of these and even run them at the same time, on separate threads
/// - You can create these on separate threads
/// - You can stop these loops without exiting your entire application
///
/// The "actual" single event loop must be created via [winit_modular::run].
/// This forwards all of its messages to the event loop using channels and returns the responses.
pub struct EventLoop {
    // id: ProxyId,
    control_flow: Arc<AtomicCell<ControlFlow>>,
    send: Sender<ProxyRequest>,
    recv: Receiver<ProxyResponse>,
    awaiting_responses: RefCell<VecDeque<PendingResponse>>,
    awaited_responses: RefCell<Vec<ReadyResponse>>,
    next_awaiting_response_id: Cell<usize>
}

impl EventLoop {
    /// Creates a new [EventLoop]. However it must first be registered, so this returns a [Future].
    pub fn new() -> FutEventLoop {
        let register_handle = Arc::new(AtomicCell::new(ProxyRegisterBody::Init));

        // SAFETY: This is already initialized and will only be read
        let sent = unsafe {
            REGISTER_PROXY.as_ref()
                .expect("you must call winit_modular::run before creating proxy event loops")
                .try_send(ProxyRegister(Arc::downgrade(&register_handle)))
        };
        match sent {
            Ok(()) => (),
            Err(TrySendError::Full(_)) => unreachable!("REGISTER_PROXY (an unbounded queue) is full?"),
            Err(TrySendError::Disconnected(_)) => panic!("main event loop crashed")
        }

        FutEventLoop {
            body: register_handle
        }
    }

    pub(crate) fn from(info: ProxyRegisterInfo) -> Self {
        EventLoop {
            // id: info.id,
            control_flow: info.control_flow,
            send: info.send,
            recv: info.recv,
            awaiting_responses: RefCell::new(VecDeque::new()),
            awaited_responses: RefCell::new(Vec::new()),
            next_awaiting_response_id: Cell::new(0)
        }
    }

    /// Creates a new [Window], using the function to add arguments
    pub fn create_window(&self, configure: impl FnOnce(WindowBuilder) -> WindowBuilder + Send + 'static) -> FutResponse<'_, Result<Window, OsError>> {
        self.send(ProxyRequest::SpawnWindow {
            configure: Box::new(configure)
        }, |response| {
            match response {
                ProxyResponse::SpawnWindow(window) => window,
                _ => panic!("incorrect response type, responses were received out-of-order")
            }
        })
    }

    fn send<T>(&self, message: ProxyRequest, convert_response: fn(ProxyResponse) -> T) -> FutResponse<'_, T> {
        match self.send.try_send(message) {
            Ok(()) => (),
            Err(TrySendError::Full(_)) => unreachable!("proxy event loop channel (unbounded) is full?"),
            Err(TrySendError::Disconnected(_)) => panic!("main event loop crashed")
        };

        let id = self.next_awaiting_response_id.get();
        self.next_awaiting_response_id.set(id + 1);
        let id = ResponseId(id);

        let mut awaiting_responses = self.awaiting_responses.borrow_mut();
        awaiting_responses.push_back(PendingResponse::new(id));

        FutResponse {
            proxy: self,
            id,
            convert: convert_response
        }
    }

    /// Receives all pending events and responses from the main loop, not blocking.
    ///
    /// You can set [ControlFlow] to exit locally or exit the app, but [ControlFlow::Wait] and [ControlFlow::WaitUntil] won't do anything.
    pub fn run_immediate(&self, mut event_handler: impl FnMut(Event, &mut ControlFlow)) {
        let mut awaiting_responses = self.awaiting_responses.borrow_mut();
        let mut awaited_responses = self.awaited_responses.borrow_mut();
        loop {
            let response = match self.recv.try_recv() {
                Ok(response) => response,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => panic!("main event loop crashed")
            };

            self.handle_response(response, &mut event_handler, &mut awaiting_responses, &mut awaited_responses);
        }
    }

    /// Receives all pending events and responses from the main loop, blocking waiting for new responses,
    /// until the event handler explicitly exits.
    pub async fn run_async(&self, mut event_handler: impl FnMut(Event, &mut ControlFlow)) {
        loop {
            let response = match self.recv.recv_async().await {
                Ok(response) => response,
                Err(_) => panic!("main event loop crashed")
            };

            // We have to borrow inside the loop because we poll on every call to .await
            let mut awaiting_responses = self.awaiting_responses.borrow_mut();
            let mut awaited_responses = self.awaited_responses.borrow_mut();

            self.handle_response(response, &mut event_handler, &mut awaiting_responses, &mut awaited_responses);
        }
    }

    /// Receives all pending events and responses from the main loop, blocking waiting for new responeses,
    /// until the event handler explicitly exits.
    pub fn run(&self, event_handler: impl FnMut(Event, &mut ControlFlow)) {
        block_on(self.run_async(event_handler))
    }

    fn handle_response(
        &self,
        response: ProxyResponse,
        event_handler: &mut impl FnMut(Event, &mut ControlFlow),
        awaiting_responses: &mut RefMut<'_, VecDeque<PendingResponse>>,
        awaited_responses: &mut RefMut<'_, Vec<ReadyResponse>>
    ) {
        if let ProxyResponse::Event(event) = response {
            let mut control_flow = self.control_flow.load();
            event_handler(event, &mut control_flow);
            self.control_flow.store(control_flow);
        } else {
            let awaiting_response = awaiting_responses.pop_front().expect("got response when we weren't awaiting one");
            awaited_responses.push(awaiting_response.wake(response));
        }
    }

    pub(crate) fn poll(&self, id: ResponseId, cx: &mut Context<'_>) -> Poll<ProxyResponse> {
        for awaiting_response in self.awaiting_responses.borrow_mut().iter_mut() {
            if awaiting_response.id == id {
                awaiting_response.poll(cx);
                return Poll::Pending;
            }
        }

        for awaited_response in self.awaited_responses.borrow_mut().drain_filter(|awaited_response| awaited_response.id == id) {
            return Poll::Ready(awaited_response.response);
        }

        panic!("unexpected response id: {}", id);
    }
}

/// Copied from [winit/event_loop](https://docs.rs/winit/0.26.1/src/winit/event_loop.rs.html) and modified.
/// See [winit::event_loop::ControlFlow docs](https://docs.rs/winit/0.26.1/winit/event_loop/enum.ControlFlow.html) for details.
///
/// Like `winit/event_loop`, the default is [Poll], but if you set the value it will persist in future
/// calls to the event handler until you set it again.
///
/// [Wait] and [WaitLocal] are supported, *but they will only actually do anything if all proxies are waiting*.
/// Otherwise you will continue to receive events as normal, so be aware.
///
/// Setting to [ExitLocal] causes the current call to [EventLoopProxy::run] or associated methods to exit,
/// while setting to [ExitApp] causes the entire application (including all other event loops) to exit.
///
/// Defaults to [`Poll`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ControlFlow {
    /// When the current loop iteration finishes, immediately begin a new iteration regardless of
    /// whether or not new events are available to process.
    Poll,
    /// When the current loop iteration finishes, suspend the thread until another event arrives,
    /// if all other proxies are waiting.
    Wait,
    /// When the current loop iteration finishes, suspend the thread until either another event
    /// arrives, the given time is reached, or another proxy is receiving events.
    ///
    /// Can be useful for implementing timers but make sure the instant is actually reached because
    /// of the "other proxies" policy.
    WaitUntil(Instant),
    /// Stop this proxy and exit the corresponding [ProxyEventLoop::run] method this event handler
    /// was registered for.
    ExitLocal,
    /// Send a [winit::events::LoopDestroyed] event and stop the event loop, stopping all other proxies.
    ExitApp
}

impl Default for ControlFlow {
    fn default() -> Self {
        ControlFlow::Poll
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SharedControlFlow {
    Wait,
    WaitUntil(Instant),
    Poll,
    ExitApp
}

impl PartialOrd for SharedControlFlow {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        match (self, other) {
            (SharedControlFlow::Wait, SharedControlFlow::Wait) => Some(Ordering::Equal),
            (SharedControlFlow::WaitUntil(a), SharedControlFlow::WaitUntil(b)) => a.partial_cmp(&b),
            (SharedControlFlow::Poll, SharedControlFlow::Poll) => Some(Ordering::Equal),
            (SharedControlFlow::ExitApp, SharedControlFlow::ExitApp) => Some(Ordering::Equal),
            (SharedControlFlow::Wait, SharedControlFlow::WaitUntil(_)) => Some(Ordering::Greater),
            (SharedControlFlow::Wait, SharedControlFlow::Poll) => Some(Ordering::Greater),
            (SharedControlFlow::Wait, SharedControlFlow::ExitApp) => Some(Ordering::Greater),
            (SharedControlFlow::WaitUntil(_), SharedControlFlow::Poll) => Some(Ordering::Greater),
            (SharedControlFlow::WaitUntil(_), SharedControlFlow::ExitApp) => Some(Ordering::Greater),
            (SharedControlFlow::Poll, SharedControlFlow::ExitApp) => Some(Ordering::Greater),
            (SharedControlFlow::WaitUntil(_), SharedControlFlow::Wait) => Some(Ordering::Less),
            (SharedControlFlow::Poll, SharedControlFlow::Wait) => Some(Ordering::Less),
            (SharedControlFlow::ExitApp, SharedControlFlow::Wait) => Some(Ordering::Less),
            (SharedControlFlow::Poll, SharedControlFlow::WaitUntil(_)) => Some(Ordering::Less),
            (SharedControlFlow::ExitApp, SharedControlFlow::WaitUntil(_)) => Some(Ordering::Less),
            (SharedControlFlow::ExitApp, SharedControlFlow::Poll) => Some(Ordering::Less),
        }
    }
}

impl Ord for SharedControlFlow {
    fn cmp(&self, other: &Self) -> Ordering {
        self.partial_cmp(other).unwrap()
    }
}
