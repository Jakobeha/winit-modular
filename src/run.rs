use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::thread::spawn;
use winit::window::WindowBuilder;
use crossbeam_utils::atomic::AtomicCell;
use flume::{TryRecvError, TrySendError, unbounded};
use crate::event_loop::{ControlFlow, SharedControlFlow};
use crate::event::{Event, UserEvent};
use crate::messages::{AppProxyRegisterInfo, ProxyRegister, ProxyRegisterBody, ProxyRegisterInfo, ProxyRequest, ProxyResponse, REGISTER_PROXY};

/// Takes control of the main thread and runs the event loop.
/// The given code will be run on a separate thread.
/// This code will be able to interact with the event loop via proxy event loops ([event_loop::EventLoop])
pub fn run(rest: impl FnOnce() + Send + 'static) -> ! {
    let (register_proxy, recv_register) = unbounded();
    // SAFETY: this is the only code which sets, and code which reads should be in threads which didn't spawn yet
    unsafe {
        REGISTER_PROXY = Some(register_proxy);
    }

    // let mut next_proxy_id = 1;
    let mut proxy_channels = Vec::new();

    EXIT_FLAG.with(|exit_flag| exit_flag.store(1, Ordering::Release));
    spawn(rest);

    winit::event_loop::EventLoop::<UserEvent>::with_user_event().run(move |event, window_target, control_flow| {
        // There is only one non-static event, ScaleFactorChanged, which is very niche. So we just ignore it.
        // We need to be able to clone the events and also send them across thread bounds
        // TODO: rename physical_size to EventOut or something and make it an enum
        // TODO: Also setting physical_size does not actually currently work due to a race condition.
        let (event, physical_size) = Event::from(event);

        // Register proxies
        for ProxyRegister(info) in recv_register.try_iter() {
            if let Some(info) = info.upgrade() {
                // let id = ProxyId(next_proxy_id);
                // next_proxy_id += 1;

                let control_flow = Arc::new(AtomicCell::new(ControlFlow::Poll));
                let (proxy_send, recv_from_proxy) = unbounded();
                let (send_to_proxy, proxy_recv) = unbounded();
                proxy_channels.push(AppProxyRegisterInfo {
                    recv_from_proxy,
                    send_to_proxy,
                    control_flow: control_flow.clone()
                });

                match info.take() {
                    ProxyRegisterBody::Init => {},
                    ProxyRegisterBody::Polled { waker } => waker.wake(),
                    ProxyRegisterBody::Ready { info: _ } => unreachable!("proxy event loop registered twice")
                }

                info.store(ProxyRegisterBody::Ready {
                    info: ProxyRegisterInfo {
                        // id,
                        control_flow,
                        send: proxy_send,
                        recv: proxy_recv,
                    }
                });
            }
        }

        // Handle proxy messages, send each proxy the event, and get their control_flow policy
        let mut shared_control_flow = SharedControlFlow::Wait;
        let mut proxy_idxs_to_remove = Vec::new();
        for (proxy_idx, AppProxyRegisterInfo { control_flow, recv_from_proxy, send_to_proxy}) in proxy_channels.iter_mut().enumerate() {
            // Handle messages
            loop {
                let request = match recv_from_proxy.try_recv() {
                    Ok(request) => request,
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        proxy_idxs_to_remove.push(proxy_idx);
                        break
                    }
                };

                let response = match request {
                    ProxyRequest::SpawnWindow { configure } => {
                        ProxyResponse::SpawnWindow { result: configure(WindowBuilder::new()).build(&window_target) }
                    }
                    ProxyRequest::RunOnMainThread { action } => {
                        ProxyResponse::RunOnMainThread { return_value: action() }
                    }
                };

                match send_to_proxy.try_send(response) {
                    Ok(_) => (),
                    Err(TrySendError::Full(_)) => unreachable!("event loop channel (unbounded) full?"),
                    Err(TrySendError::Disconnected(_)) => {
                        proxy_idxs_to_remove.push(proxy_idx);
                        break
                    }
                }
            }

            // Send the event
            match send_to_proxy.try_send(ProxyResponse::Event(event.clone())) {
                Ok(_) => (),
                Err(TrySendError::Full(_)) => unreachable!("event loop channel (unbounded) full?"),
                Err(TrySendError::Disconnected(_)) => proxy_idxs_to_remove.push(proxy_idx)
            }

            // Get control flow policy
            match control_flow.load() {
                ControlFlow::Poll => shared_control_flow = shared_control_flow.min(SharedControlFlow::Poll),
                ControlFlow::Wait => shared_control_flow = shared_control_flow.min(SharedControlFlow::Wait),
                ControlFlow::WaitUntil(instant) => shared_control_flow = shared_control_flow.min(SharedControlFlow::WaitUntil(instant)),
                ControlFlow::ExitLocal => {
                    // proxy exits itself, if it actually gets dropped we will remove but it may run again
                }
                ControlFlow::ExitApp => shared_control_flow = shared_control_flow.min(SharedControlFlow::ExitApp),
            }
        }

        // Remove disconnected proxies
        for proxy_to_remove in proxy_idxs_to_remove.into_iter().rev() {
            proxy_channels.remove(proxy_to_remove);
        }

        // Update event and control flow
        event.into(physical_size);
        *control_flow = match shared_control_flow {
            SharedControlFlow::Wait => winit::event_loop::ControlFlow::Wait,
            SharedControlFlow::Poll => winit::event_loop::ControlFlow::Poll,
            SharedControlFlow::WaitUntil(instant) => winit::event_loop::ControlFlow::WaitUntil(instant),
            SharedControlFlow::ExitApp => winit::event_loop::ControlFlow::Exit,
        };

        if EXIT_FLAG.with(|exit_flag| exit_flag.load(Ordering::Acquire)) == 2 {
            *control_flow = winit::event_loop::ControlFlow::Exit;
        }
    })
}

/// Forces the program to exit via winit's event loop.
///
/// If [run] is not called before this it exits normally.
pub fn exit() {
    if EXIT_FLAG.with(|exit_flag| exit_flag.load(Ordering::Acquire)) == 0 {
        std::process::exit(0);
    }
    EXIT_FLAG.with(|exit_flag| exit_flag.store(2, Ordering::Release));
}

thread_local! {
    static EXIT_FLAG: Arc<AtomicU8> = Arc::new(AtomicU8::new(0));
}