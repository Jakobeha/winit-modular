use std::any::Any;
use std::cmp::Ordering;
use std::sync::Arc;
use crossbeam_utils::atomic::AtomicCell;
use flume::{Receiver, Sender, TryRecvError, TrySendError};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use winit::window::{Window, WindowBuilder};
use winit::error::OsError;
use futures::executor::block_on;
use std::task::Waker;
use std::time::Instant;
use crate::event::Event;
use crate::future::{FutResponse, PendingRequest, FutEventLoop};
use crate::messages::{ProxyRegister, ProxyRegisterBody, ProxyRegisterInfo, ProxyRequest, ProxyResponse, REGISTER_PROXY};

/// A proxy event loop.
///
/// Similar API to [winit::event_loop::EventLoop], except:
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
    pending_requests: RefCell<VecDeque<PendingRequest>>,
    locally_pending_events: RefCell<Vec<Event>>,
    is_receiving_events: Cell<bool>
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[doc(hidden)]
/// Whether an event is during or before the call to [EventLoop::run] or [EventLoop::run_async]
pub enum EventIs {
    /// Event was before the call to `run...`
    Buffered,
    /// Event was during the call to `run...`
    New
}

impl EventLoop {
    /// Creates a new proxy event loop. However it must first be registered, so this is async.
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
            pending_requests: RefCell::new(VecDeque::new()),
            locally_pending_events: RefCell::new(Vec::new()),
            is_receiving_events: Cell::new(false)
        }
    }

    /// Runs an arbitrary closure on the main / UI thread.
    ///
    /// Note that the closure must be `'static`, which means it can't reference local variables.
    ///
    /// To get around this, you can convert them to raw pointers, wrap them in an `unsafe Send` struct,
    /// and `unsafe` dereference them inside the closure. This should actually be 100% safe as long
    /// as you `await` and do not drop the `Future`, as those references will remain alive and won't
    /// be dereferenced outside of the closure until the future ends.
    ///
    /// Alternatively, to get around this without `unsafe`, you can `move` the local variables into
    /// the closure and then return them along with your "real" result.
    ///
    /// In the future, we may provide more methods to work around this limitation.
    pub fn on_main_thread<R: Any + Send>(&self, action: impl FnOnce() -> R + Send + 'static) -> FutResponse<'_, R> {
        self.send(ProxyRequest::RunOnMainThread {
            action: Box::new(move || Box::new(action()))
        }, |response| {
            match response {
                ProxyResponse::RunOnMainThread { return_value } => {
                    *return_value.downcast::<R>().expect("incorrect return value type, responses were received out-of-order")
                }
                _ => panic!("incorrect response type, responses were received out-of-order")
            }
        })
    }
    /// Creates a new [Window], using the function to add arguments
    pub fn create_window(&self, configure: impl FnOnce(WindowBuilder) -> WindowBuilder + Send + 'static) -> FutResponse<'_, Result<Window, OsError>> {
        self.send(ProxyRequest::SpawnWindow {
            configure: Box::new(configure)
        }, |response| {
            match response {
                ProxyResponse::SpawnWindow { result } => result,
                _ => panic!("incorrect response type, responses were received out-of-order")
            }
        })
    }

    /// Receives new *and buffered* events and responses from the main loop, blocking waiting for new responses,
    /// until the event handler explicitly exits.
    ///
    /// The third argument to `event_handler` is whether the event is buffered (i.e. sent before this was called) or new.
    pub fn run(&self, event_handler: impl FnMut(Event, &mut ControlFlow, EventIs)) {
        block_on(self.run_async(event_handler))
    }

    /// Receives new *and buffered* events and responses from the main loop, blocking waiting for new responses,
    /// until the event handler explicitly exits.
    ///
    /// The third argument to `event_handler` is whether the event is buffered (i.e. sent before this was called) or new.
    pub async fn run_async(&self, mut event_handler: impl FnMut(Event, &mut ControlFlow, EventIs)) {
        assert!(!self.is_receiving_events.get(), "already running");
        self.is_receiving_events.set(true);
        // Handle locally pending events
        for event in self.locally_pending_events.borrow_mut().drain(..) {
            match self.handle_event(event, |event, control_flow| {
                event_handler(event, control_flow, EventIs::New)
            }) {
                std::ops::ControlFlow::Break(()) => {
                    // Exit early
                    self.is_receiving_events.set(false);
                    break
                },
                std::ops::ControlFlow::Continue(()) => ()
            }
        }
        // Handle remote pending and new events
        self._run_async(event_handler).await;
        self.is_receiving_events.set(false);
    }

    async fn run_only_responses(&self) {
        if !self.is_receiving_events.get() {
            self._run_async(|_, _, _| unreachable!("called event handler but we are not receiving events")).await;
        }
    }

    /// Receives new *and buffered* events and responses from the main loop, blocking waiting for new responses,
    /// until the event handler explicitly exits.
    ///
    /// The third argument to `event_handler` is whether the event is buffered (i.e. sent before this was called) or new.
    async fn _run_async(&self, mut event_handler: impl FnMut(Event, &mut ControlFlow, EventIs)) {
        // Handle pending events
        self.run_immediate(|event, control_flow| {
            event_handler(event, control_flow, EventIs::Buffered);
        });

        // Handle new events
        loop {
            let response = match self.recv.recv_async().await {
                Ok(response) => response,
                Err(_) => panic!("main event loop crashed")
            };

            match self.handle_response(response, |event, control_flow| {
                event_handler(event, control_flow, EventIs::New)
            }) {
                std::ops::ControlFlow::Break(()) => break,
                std::ops::ControlFlow::Continue(()) => ()
            }
        }
    }

    /// Receives all buffered events and responses from the main loop, not blocking for new events.
    ///
    /// You can set [ControlFlow] to exit locally or exit the app, but [ControlFlow::Wait] and [ControlFlow::WaitUntil] won't do anything.
    pub fn run_immediate(&self, mut event_handler: impl FnMut(Event, &mut ControlFlow)) {
        loop {
            let response = match self.recv.try_recv() {
                Ok(response) => response,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => panic!("main event loop crashed")
            };

            match self.handle_response(response, &mut event_handler) {
                std::ops::ControlFlow::Break(()) => break,
                std::ops::ControlFlow::Continue(()) => ()
            }
        }
    }

    fn handle_response(
        &self,
        response: ProxyResponse,
        event_handler: impl FnMut(Event, &mut ControlFlow)
    ) -> std::ops::ControlFlow<()> {
        // Events are separate from "regular" responses.
        // Events we just forward to the event handler,
        // other responses are associated with requests which need them in order to be resolved.
        // So the algorithm is:
        // - If this is an event, forward to the event handler
        // - Else there should be a pending request, resolve it
        if self.is_receiving_events.get() {
            if let ProxyResponse::Event(event) = response {
                self.handle_event(event, event_handler)
            } else if let Some(pending_request) = self.pending_requests.borrow_mut().pop_front() {
                pending_request.resolve(response);
                std::ops::ControlFlow::Continue(())
            } else {
                panic!("unhandled response with no associated request (is_receiving_events = true)");
            }
        } else {
            let mut pending_requests = self.pending_requests.borrow_mut();
            if let ProxyResponse::Event(event) = response {
                self.locally_pending_events.borrow_mut().push(event);
                std::ops::ControlFlow::Continue(())
            } else if let Some(pending_request) = pending_requests.pop_front() {
                pending_request.resolve(response);
                if pending_requests.is_empty() {
                    // Only meant to receive responses, and we are done receiving them
                    std::ops::ControlFlow::Break(())
                } else {
                    std::ops::ControlFlow::Continue(())
                }
            } else {
                panic!("unhandled response with no associated request (is_receiving_events = false)");
            }
        }
    }

    fn handle_event(&self, event: Event, mut event_handler: impl FnMut(Event, &mut ControlFlow)) -> std::ops::ControlFlow<()> {
        let mut control_flow = self.control_flow.load();
        debug_assert_ne!(control_flow, ControlFlow::ExitLocal);
        event_handler(event, &mut control_flow);
        if control_flow == ControlFlow::ExitLocal {
            std::ops::ControlFlow::Break(())
        } else {
            self.control_flow.store(control_flow);
            std::ops::ControlFlow::Continue(())
        }
    }

    fn send<T>(&self, message: ProxyRequest, convert_response: fn(ProxyResponse) -> T) -> FutResponse<'_, T> {
        FutResponse::new(self, message, convert_response)
    }

    pub(crate) async fn actually_send(&self, message: ProxyRequest, waker: Waker, response_ptr: *mut Option<ProxyResponse>) {
        match self.send.try_send(message) {
            Ok(()) => (),
            Err(TrySendError::Full(_)) => unreachable!("proxy event loop channel (unbounded) is full?"),
            Err(TrySendError::Disconnected(_)) => panic!("main event loop crashed")
        };

        self.pending_requests.borrow_mut().push_back(PendingRequest::new(waker, response_ptr));

        self.run_only_responses().await;
    }
}

/// [winit::event_loop::ControlFlow] for a proxy event loop.
///
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
