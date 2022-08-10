//! Copied directly from [winit/event](https://docs.rs/winit/0.26.1/src/winit/event.rs.html),
//! modified to support passing events to multiple proxies. See the
//! [event documentation](https://docs.rs/winit/0.26.1/winit/event/enum.Event.html) for details.
//!
//! Every single [winit::event::Event](winit::event::Event) has a corresponding
//! [winit_modular::event::Event](winit::event::Event), except that [ProxyEvent]s do not have a
//! lifetime so they can be sent across threads.
//!
//! Different proxies may have different custom event implementations. As a result we decided to
//! make 2 kinds of custom events: `Box<dyn Any>`, and if you care about allocation, `usize`,
//! can be sent as custom events. Custom events are one of the ways to communicate across [ProxyEventLoop]s.
use std::any::Any;
use std::fmt::Debug;
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{AxisId, DeviceEvent, DeviceId, ElementState, KeyboardInput, ModifiersState, MouseButton, MouseScrollDelta, StartCause, Touch, TouchPhase};
use winit::window::{Theme, WindowId};

/// Event which gets sent to proxy [EventLoop]s. See [winit::event::Event] for details.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Emitted when new events arrive from the OS to be processed.
    ///
    /// This event type is useful as a place to put code that should be done before you start
    /// processing events, such as updating frame timing information for benchmarking or checking
    /// the [`StartCause`][crate::event::StartCause] to see if a timer set by
    /// [`ControlFlow::WaitUntil`](crate::event_loop::ControlFlow::WaitUntil) has elapsed.
    NewEvents(StartCause),

    /// Emitted when the OS sends an event to a winit window.
    WindowEvent {
        window_id: WindowId,
        event: WindowEvent,
    },

    /// Emitted when the OS sends an event to a device.
    DeviceEvent {
        device_id: DeviceId,
        event: DeviceEvent,
    },

    /// Emitted when an event is sent from [`EventLoopProxy::send_event`](crate::event_loop::EventLoopProxy::send_event)
    UserEvent(UserEvent),

    /// Emitted when the application has been suspended.
    Suspended,

    /// Emitted when the application has been resumed.
    Resumed,

    /// Emitted when all of the event loop's input events have been processed and redraw processing
    /// is about to begin.
    ///
    /// This event is useful as a place to put your code that should be run after all
    /// state-changing events have been handled and you want to do stuff (updating state, performing
    /// calculations, etc) that happens as the "main body" of your event loop. If your program only draws
    /// graphics when something changes, it's usually better to do it in response to
    /// [`Event::RedrawRequested`](crate::event::Event::RedrawRequested), which gets emitted
    /// immediately after this event. Programs that draw graphics continuously, like most games,
    /// can render here unconditionally for simplicity.
    MainEventsCleared,

    /// Emitted after `MainEventsCleared` when a window should be redrawn.
    ///
    /// This gets triggered in two scenarios:
    /// - The OS has performed an operation that's invalidated the window's contents (such as
    ///   resizing the window).
    /// - The application has explicitly requested a redraw via
    ///   [`Window::request_redraw`](crate::window::Window::request_redraw).
    ///
    /// During each iteration of the event loop, Winit will aggregate duplicate redraw requests
    /// into a single event, to help avoid duplicating rendering work.
    ///
    /// Mainly of interest to applications with mostly-static graphics that avoid redrawing unless
    /// something changes, like most non-game GUIs.
    RedrawRequested(WindowId),

    /// Emitted after all `RedrawRequested` events have been processed and control flow is about to
    /// be taken away from the program. If there are no `RedrawRequested` events, it is emitted
    /// immediately after `MainEventsCleared`.
    ///
    /// This event is useful for doing any cleanup or bookkeeping work after all the rendering
    /// tasks have been completed.
    RedrawEventsCleared,

    /// Emitted when the event loop is being shut down.
    ///
    /// This is irreversible - if this event is emitted, it is guaranteed to be the last event that
    /// gets emitted. You generally want to treat this as an "do on quit" event.
    LoopDestroyed,
}

/// Describes an event from a [winit::window::Window]. See [winit::event::WindowEvent] for details.
#[derive(Debug, Clone, PartialEq)]
pub enum WindowEvent {
    /// The size of the window has changed. Contains the client area's new dimensions.
    Resized(PhysicalSize<u32>),

    /// The position of the window has changed. Contains the window's new position.
    Moved(PhysicalPosition<i32>),

    /// The window has been requested to close.
    CloseRequested,

    /// The window has been destroyed.
    Destroyed,

    /// A file has been dropped into the window.
    ///
    /// When the user drops multiple files at once, this event will be emitted for each file
    /// separately.
    DroppedFile(PathBuf),

    /// A file is being hovered over the window.
    ///
    /// When the user hovers multiple files at once, this event will be emitted for each file
    /// separately.
    HoveredFile(PathBuf),

    /// A file was hovered, but has exited the window.
    ///
    /// There will be a single `HoveredFileCancelled` event triggered even if multiple files were
    /// hovered.
    HoveredFileCancelled,

    /// The window received a unicode character.
    ReceivedCharacter(char),

    /// The window gained or lost focus.
    ///
    /// The parameter is true if the window has gained focus, and false if it has lost focus.
    Focused(bool),

    /// An event from the keyboard has been received.
    KeyboardInput {
        device_id: DeviceId,
        input: KeyboardInput,
        /// If `true`, the event was generated synthetically by winit
        /// in one of the following circumstances:
        ///
        /// * Synthetic key press events are generated for all keys pressed
        ///   when a window gains focus. Likewise, synthetic key release events
        ///   are generated for all keys pressed when a window goes out of focus.
        ///   ***Currently, this is only functional on X11 and Windows***
        ///
        /// Otherwise, this value is always `false`.
        is_synthetic: bool,
    },

    /// The keyboard modifiers have changed.
    ///
    /// Platform-specific behavior:
    /// - **Web**: This API is currently unimplemented on the web. This isn't by design - it's an
    ///   issue, and it should get fixed - but it's the current state of the API.
    ModifiersChanged(ModifiersState),

    /// The cursor has moved on the window.
    CursorMoved {
        device_id: DeviceId,

        /// (x,y) coords in pixels relative to the top-left corner of the window. Because the range of this data is
        /// limited by the display area and it may have been transformed by the OS to implement effects such as cursor
        /// acceleration, it should not be used to implement non-cursor-like interactions such as 3D camera control.
        position: PhysicalPosition<f64>,
        #[deprecated = "Deprecated in favor of WindowEvent::ModifiersChanged"]
        modifiers: ModifiersState,
    },

    /// The cursor has entered the window.
    CursorEntered { device_id: DeviceId },

    /// The cursor has left the window.
    CursorLeft { device_id: DeviceId },

    /// A mouse wheel movement or touchpad scroll occurred.
    MouseWheel {
        device_id: DeviceId,
        delta: MouseScrollDelta,
        phase: TouchPhase,
        #[deprecated = "Deprecated in favor of WindowEvent::ModifiersChanged"]
        modifiers: ModifiersState,
    },

    /// An mouse button press has been received.
    MouseInput {
        device_id: DeviceId,
        state: ElementState,
        button: MouseButton,
        #[deprecated = "Deprecated in favor of WindowEvent::ModifiersChanged"]
        modifiers: ModifiersState,
    },

    /// Touchpad pressure event.
    ///
    /// At the moment, only supported on Apple forcetouch-capable macbooks.
    /// The parameters are: pressure level (value between 0 and 1 representing how hard the touchpad
    /// is being pressed) and stage (integer representing the click level).
    TouchpadPressure {
        device_id: DeviceId,
        pressure: f32,
        stage: i64,
    },

    /// Motion on some analog axis. May report data redundant to other, more specific events.
    AxisMotion {
        device_id: DeviceId,
        axis: AxisId,
        value: f64,
    },

    /// Touch event has been received
    Touch(Touch),

    /// The window's scale factor has changed.
    ///
    /// The following user actions can cause DPI changes:
    ///
    /// * Changing the display's resolution.
    /// * Changing the display's scale factor (e.g. in Control Panel on Windows).
    /// * Moving the window to a display with a different scale factor.
    ///
    /// After this event callback has been processed, the window will be resized to whatever value
    /// is pointed to by the `new_inner_size` reference. By default, this will contain the size suggested
    /// by the OS, but it can be changed to any value.
    ///
    /// For more information about DPI in general, see the [`dpi`](crate::dpi) module.
    ScaleFactorChanged {
        scale_factor: f64,
        new_inner_size: NewInnerSize,
    },

    /// The system window theme has changed.
    ///
    /// Applications might wish to react to this to change the theme of the content of the window
    /// when the system changes the window theme.
    ///
    /// At the moment this is only supported on Windows.
    ThemeChanged(Theme),
}

/// Allows you to set the inner size in a `WindowEvent::ScaleFactorChanged` event,
/// but it doesn't actually work yet.
#[derive(Debug, Clone)]
#[doc(hidden)]
pub struct NewInnerSize(Arc<Mutex<PhysicalSize<u32>>>);

impl Deref for NewInnerSize {
    type Target = Mutex<PhysicalSize<u32>>;

    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

impl PartialEq for NewInnerSize {
    fn eq(&self, _other: &Self) -> bool {
        // assumes they are equal
        true
    }
}

/// A generic custom event which should support most use cases.
///
/// Since there is no shared event loop and anyone can create a proxy, the type of user events is dynamic.
#[derive(Debug, Clone)]
pub enum UserEvent {
    Primitive(usize),
    Box(Box<dyn UserEventTrait>),
}

/// Traits which custom events must implement.
///
/// Custom events must support testing for equality because [Event] is.
/// If you don't need this you can create a dummy implementation which always returns `false`.
/// They must implement cloning and [Send] because they will get cloned and sent to each proxy.
/// They must implement [Any] so they can be downcasted.
pub trait UserEventTrait: Any + Debug + Send {
    /// Custom events must support testing for equality because [Event] is.
    /// If you don't need this you can create a dummy implementation which always returns `false`.
    fn rough_eq(&self, other: &dyn UserEventTrait) -> bool;
    fn clone(&self) -> Box<dyn UserEventTrait>;
}

impl Clone for Box<dyn UserEventTrait> {
    fn clone(&self) -> Self {
        UserEventTrait::clone(self.as_ref())
    }
}

impl PartialEq for Box<dyn UserEventTrait> {
    fn eq(&self, other: &Self) -> bool {
        UserEventTrait::rough_eq(self.as_ref(), other.as_ref())
    }
}

impl PartialEq for UserEvent {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (UserEvent::Primitive(a), UserEvent::Primitive(b)) => a == b,
            (UserEvent::Box(a), UserEvent::Box(b)) => a.eq(b),
            _ => false,
        }
    }
}

#[allow(deprecated)]
impl Event {
    pub fn from(event: winit::event::Event<'_, UserEvent>) -> (Self, Option<&mut PhysicalSize<u32>>) {
        match event {
            winit::event::Event::NewEvents(x) => (Event::NewEvents(x.clone()), None),
            winit::event::Event::WindowEvent { window_id, event } => {
                let (event, physical_size) = WindowEvent::from(event);
                (Event::WindowEvent { window_id: window_id.clone(), event }, physical_size)
            },
            winit::event::Event::DeviceEvent { device_id, event } => (Event::DeviceEvent { device_id: device_id.clone(), event: event.clone() }, None),
            winit::event::Event::UserEvent(x) => (Event::UserEvent(x.clone()), None),
            winit::event::Event::Suspended => (Event::Suspended, None),
            winit::event::Event::Resumed => (Event::Resumed, None),
            winit::event::Event::MainEventsCleared => (Event::MainEventsCleared, None),
            winit::event::Event::RedrawRequested(x) => (Event::RedrawRequested(x.clone()), None),
            winit::event::Event::RedrawEventsCleared => (Event::RedrawEventsCleared, None),
            winit::event::Event::LoopDestroyed => (Event::LoopDestroyed, None)
        }
    }

    pub fn into(self, physical_size: Option<&mut PhysicalSize<u32>>) -> winit::event::Event<'_, UserEvent> {
        match self {
            Event::NewEvents(x) => winit::event::Event::NewEvents(x),
            Event::WindowEvent { window_id, event } => {
                let event = event.into(physical_size);
                winit::event::Event::WindowEvent { window_id, event }
            }
            Event::DeviceEvent { device_id, event } => winit::event::Event::DeviceEvent { device_id, event },
            Event::UserEvent(x) => winit::event::Event::UserEvent(x),
            Event::Suspended => winit::event::Event::Suspended,
            Event::Resumed => winit::event::Event::Resumed,
            Event::MainEventsCleared => winit::event::Event::MainEventsCleared,
            Event::RedrawRequested(x) => winit::event::Event::RedrawRequested(x),
            Event::RedrawEventsCleared => winit::event::Event::RedrawEventsCleared,
            Event::LoopDestroyed => winit::event::Event::LoopDestroyed
        }
    }
}

#[allow(deprecated)]
impl WindowEvent {
    pub fn from(event: winit::event::WindowEvent<'_>) -> (Self, Option<&mut PhysicalSize<u32>>) {
        match event {
            winit::event::WindowEvent::Resized(x) => (WindowEvent::Resized(x), None),
            winit::event::WindowEvent::Moved(x) => (WindowEvent::Moved(x), None),
            winit::event::WindowEvent::CloseRequested => (WindowEvent::CloseRequested, None),
            winit::event::WindowEvent::Destroyed => (WindowEvent::Destroyed, None),
            winit::event::WindowEvent::DroppedFile(x) => (WindowEvent::DroppedFile(x), None),
            winit::event::WindowEvent::HoveredFile(x) => (WindowEvent::HoveredFile(x), None),
            winit::event::WindowEvent::HoveredFileCancelled => (WindowEvent::HoveredFileCancelled, None),
            winit::event::WindowEvent::ReceivedCharacter(x) => (WindowEvent::ReceivedCharacter(x), None),
            winit::event::WindowEvent::Focused(x) => (WindowEvent::Focused(x), None),
            winit::event::WindowEvent::KeyboardInput { device_id, input, is_synthetic } => (WindowEvent::KeyboardInput { device_id, input, is_synthetic }, None),
            winit::event::WindowEvent::ModifiersChanged(x) => (WindowEvent::ModifiersChanged(x), None),
            winit::event::WindowEvent::CursorMoved { device_id, position, modifiers } => (WindowEvent::CursorMoved { device_id, position, modifiers }, None),
            winit::event::WindowEvent::CursorEntered { device_id } => (WindowEvent::CursorEntered { device_id }, None),
            winit::event::WindowEvent::CursorLeft { device_id } => (WindowEvent::CursorLeft { device_id }, None),
            winit::event::WindowEvent::MouseWheel { device_id, delta, modifiers, phase } => (WindowEvent::MouseWheel { device_id, delta, modifiers, phase }, None),
            winit::event::WindowEvent::MouseInput { device_id, modifiers, state, button } => (WindowEvent::MouseInput { device_id, modifiers, state, button }, None),
            winit::event::WindowEvent::TouchpadPressure { device_id, pressure, stage } => (WindowEvent::TouchpadPressure { device_id, pressure, stage }, None),
            winit::event::WindowEvent::AxisMotion { device_id, axis, value } => (WindowEvent::AxisMotion { device_id, axis, value }, None),
            winit::event::WindowEvent::Touch(x) => (WindowEvent::Touch(x), None),
            winit::event::WindowEvent::ScaleFactorChanged { scale_factor, new_inner_size } => (WindowEvent::ScaleFactorChanged { scale_factor, new_inner_size: NewInnerSize(Arc::new(Mutex::new(*new_inner_size))) }, Some(new_inner_size)),
            winit::event::WindowEvent::ThemeChanged(x) => (WindowEvent::ThemeChanged(x), None),
        }
    }

    pub fn into(self, physical_size: Option<&mut PhysicalSize<u32>>) -> winit::event::WindowEvent<'_> {
        match self {
            WindowEvent::Resized(x) => winit::event::WindowEvent::Resized(x),
            WindowEvent::Moved(x) => winit::event::WindowEvent::Moved(x),
            WindowEvent::CloseRequested => winit::event::WindowEvent::CloseRequested,
            WindowEvent::Destroyed => winit::event::WindowEvent::Destroyed,
            WindowEvent::DroppedFile(x) => winit::event::WindowEvent::DroppedFile(x),
            WindowEvent::HoveredFile(x) => winit::event::WindowEvent::HoveredFile(x),
            WindowEvent::HoveredFileCancelled => winit::event::WindowEvent::HoveredFileCancelled,
            WindowEvent::ReceivedCharacter(x) => winit::event::WindowEvent::ReceivedCharacter(x),
            WindowEvent::Focused(x) => winit::event::WindowEvent::Focused(x),
            WindowEvent::KeyboardInput { device_id, input, is_synthetic } => winit::event::WindowEvent::KeyboardInput { device_id, input, is_synthetic },
            WindowEvent::ModifiersChanged(x) => winit::event::WindowEvent::ModifiersChanged(x),
            WindowEvent::CursorMoved { device_id, position, modifiers } => winit::event::WindowEvent::CursorMoved { device_id, position, modifiers },
            WindowEvent::CursorEntered { device_id } => winit::event::WindowEvent::CursorEntered { device_id },
            WindowEvent::CursorLeft { device_id } => winit::event::WindowEvent::CursorLeft { device_id },
            WindowEvent::MouseWheel { device_id, delta, modifiers, phase } => winit::event::WindowEvent::MouseWheel { device_id, delta, modifiers, phase },
            WindowEvent::MouseInput { device_id, modifiers, state, button } => winit::event::WindowEvent::MouseInput { device_id, modifiers, state, button },
            WindowEvent::TouchpadPressure { device_id, pressure, stage } => winit::event::WindowEvent::TouchpadPressure { device_id, pressure, stage },
            WindowEvent::AxisMotion { device_id, axis, value } => winit::event::WindowEvent::AxisMotion { device_id, axis, value },
            WindowEvent::Touch(x) => winit::event::WindowEvent::Touch(x),
            WindowEvent::ScaleFactorChanged { scale_factor, new_inner_size } => {
                let physical_size = physical_size.unwrap();
                if let Ok(new_inner_size) = new_inner_size.lock() {
                    *physical_size = *new_inner_size;
                }
                winit::event::WindowEvent::ScaleFactorChanged { scale_factor, new_inner_size: physical_size }
            },
            WindowEvent::ThemeChanged(x) => winit::event::WindowEvent::ThemeChanged(x)
        }
    }
}