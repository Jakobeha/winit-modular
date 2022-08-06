use std::collections::HashMap;
use winit::event_loop::EventLoop;
use lazy_static::{lazy_static, LazyStatic};
use std::sync::Arc;
use crossbeam_channel::{Sender, Receiver, unbounded, TrySendError};
use std::thread::spawn;
use winit::error::OsError;
use winit::event::{Event, StartCause};
use winit::window::{Window, WindowBuilder};
use std::cell::Cell;
use std::rc::{Rc, Weak};
use std::sync::atomic::AtomicUsize;

#[derie(Debug)]
struct EventLoopChannelCrashed;

#[derive(Debug)]
struct ProxyMessage<T> {
    message_id: ProxyMessageId,
    proxy_id: ProxyEventLoopId,
    body: ProxyMessageBody
}

type ProxyRequest = ProxyMessage<ProxyRequestBody>;
type ProxyResponse = ProxyMessage<ProxyResponseBody>;

#[derive(Debug)]
enum ProxyRequestBody {
    SpawnWindow {
        configure: Box<dyn FnOnce(WindowBuilder) -> WindowBuilder + Send>
    }
}

#[derive(Debug)]
enum ProxyResponseBody {
    SpawnWindow(Result<Window, OsError>)
}

pub struct AwaitingResponseUntyped {

}

pub struct AwaitingResponse<T> {
    base: Rc<AwaitingResponseUntyped>,
    convert: fn(ProxyResponseBody) -> T
}

/// Similar API to [EventLoop], however
///
/// - You can create multiples of these and even run them at the same time, on separate threads
/// - You can create these on separate threads
/// - You can stop these loops without exiting your entire application
///
/// The "actual" single event loop must be created via [winit_modular::run].
/// This forwards all of its messages to the event loop using channels and returns the responses.
pub struct ProxyEventLoop {
    id: ProxyEventLoopId,
    awaiting_responses: HashMap<MessageId, Weak<AwaitingResponseUntyped>>
}

impl ProxyEventLoop {
    /// Creates a new [LocalEventLoop]. This will initialize [SharedEventLoop] if not already.
    pub fn new() -> Self {
        ProxyEventLoop {
            id: ProxyEventLoopId::next(),
            awaiting_responses: HashMap::new()
        }
    }

    /// Creates a new [Window], using the function to add arguments
    pub fn create_window(&mut self, configure: impl FnOnce(WindowBuilder) -> WindowBuilder + Send) -> Result<AwaitingResponse<Result<Window, OsError>>, EventLoopChannelCrashed> {
        self.send(ProxyRequest::SpawnWindow {
            configure: Box::new(configure)
        }, |response| {
            match response {
                ProxyResponseBody::SpawnWindow(window) => window,
                _ => panic!("unexpected response: {:?}", response)
            }
        })
    }

    fn send<T>(&mut self, body: ProxyRequestBody, convert_response: fn(ProxyResponseBody) -> T) -> Result<AwaitingResponse<T>, EventLoopChannelCrashed> {
        let proxy_send = Self::send_channel();

        let message_id = MessageId::next();
        let message = ProxyMessage {
            message_id,
            proxy_id: self.id,
            body
        };

        let awaiting_response = Rc::new(AwaitingResponseUntyped {});
        self.awaiting_responses.insert(message_id, Rc::downgrade(&awaiting_response));

        match proxy_send.try_send(message) {
            Ok(()) => Ok(AwaitingResponse {
                base: awaiting_response,
                convert: convert_response
            }),
            Err(TrySendError::Full(_)) => unreachable!("proxy event loop channel (unbounded) is full?"),
            Err(TrySendError::Disconnected(_)) => Err(EventLoopChannelCrashed)
        }
    }

    fn send_channel<'a>() -> &'a Sender<ProxyRequest> {
        // SAFETY: This was set before the current thread was spawned,
        // and does't get modified after they are set
        unsafe { &PROXY_SEND.expect("proxy channels not initialized") }
    }

    fn recv_channel<'a>() -> &'a Receiver<ProxyResponse> {
        unsafe { &PROXY_RECV.expect("proxy channels not initialized") }
    }
}

/// Takes control of the main thread and runs the event loop.
/// The given code will be run on a separate thread.
/// This code will be able to interact with the event loop via [EventLoopHandle]
pub fn run(rest: impl FnOnce() + Send + 'static) -> ! {
    let (proxy_send, recv_proxy) = unbounded();
    let (send_proxy, proxy_recv) = unbounded();
    // SAFETY: this is the only code which sets, and code which reads should be in threads which didn't spawn yet
    unsafe {
        PROXY_SEND = Some(proxy_send);
        PROXY_RECV = Some(proxy_recv);
    }

    spawn(rest);

    EventLoop::new().run(move |event, window_target, control_flow| {
        for ProxyMessage { message_id, proxy_id, body: request_body } in recv_proxy.try_iter() {
            let response_body = match request_body {
                ProxyRequestBody::SpawnWindow { configure } => {
                    ProxyResponseBody::SpawnWindow(configure(WindowBuilder::new()).build(&event_loop))
                }
            };
            let response = ProxyResponse {
                message_id,
                proxy_id,
                body: response_body
            };

            match send_proxy.try_send(response) {
                Ok(_) => (),
                Err(TrySendError::Full(_)) => unreachable!("event loop channel (unbounded) full?"),
                Err(TrySendError::Disconnected(_)) => panic!("event loop channel crashed")
            };
        }
    })
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
struct ProxyEventLoopId(usize);

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
struct MessageId(usize);

static mut PROXY_SEND: Option<Sender<ProxyRequest>> = None;
static mut PROXY_RECV: Option<Receiver<ProxyResponse>> = None;

static NEXT_PROXY_ID: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static MESSAGE_ID: Cell<usize> = Cell::new(0);
}

impl MessageId {
    fn next() -> MessageId {
        MESSAGE_ID.with(|message_id| {
            let id = message_id.get();
            message_id.set(id + 1);
            MessageId(id)
        })
    }
}

impl ProxyEventLoopId {
    fn next() -> ProxyEventLoopId {
        let idx = NEXT_PROXY_ID.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        ProxyEventLoopId(idx)
    }
}