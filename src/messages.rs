use std::any::Any;
use winit::window::{Window, WindowBuilder};
use winit::error::OsError;
use flume::{Receiver, Sender};
use std::sync::{Arc, Weak};
use crossbeam_utils::atomic::AtomicCell;
use std::task::Waker;
use crate::event::Event;
use crate::event_loop::ControlFlow;

pub(crate) enum ProxyRequest {
    SpawnWindow {
        configure: Box<dyn FnOnce(WindowBuilder) -> WindowBuilder + Send>
    },
    RunOnMainThread {
        action: Box<dyn FnOnce() -> Box<dyn Any> + Send>
    }
}

pub(crate) enum ProxyResponse {
    SpawnWindow { result: Result<Window, OsError> },
    RunOnMainThread { return_value: Box<dyn Any> },
    Event(Event)
}

pub(crate) struct ProxyRegister(pub(crate) Weak<AtomicCell<ProxyRegisterBody>>);

pub(crate) enum ProxyRegisterBody {
    Init,
    Polled { waker: Waker },
    Ready { info: ProxyRegisterInfo }
}

impl Default for ProxyRegisterBody {
    fn default() -> Self {
        ProxyRegisterBody::Init
    }
}

pub(crate) struct ProxyRegisterInfo {
    // pub(crate) id: ProxyId,
    pub(crate) control_flow: Arc<AtomicCell<ControlFlow>>,
    pub(crate) send: Sender<ProxyRequest>,
    pub(crate) recv: Receiver<ProxyResponse>
}

pub(crate) struct AppProxyRegisterInfo {
    // pub(crate) id: ProxyId,
    pub(crate) control_flow: Arc<AtomicCell<ControlFlow>>,
    pub(crate) recv_from_proxy: Receiver<ProxyRequest>,
    pub(crate) send_to_proxy: Sender<ProxyResponse>,
}

pub(crate) static mut REGISTER_PROXY: Option<Sender<ProxyRegister>> = None;
