# winit-modular: proxy `winit` event loops which can be run at the same time, on separate threads

[![](https://docs.rs/winit-modular/badge.svg)](https://docs.rs/winit-modular/)
[![](https://img.shields.io/crates/v/winit-modular.svg)](https://crates.io/crates/winit-modular)
[![](https://img.shields.io/crates/d/winit-modular.svg)](https://crates.io/crates/winit-modular)

Provides an API very similar to `winit` except the `EventLoop` type can be created on multiple threads, multiple event loops can exist simultaneously, you can poll for events or receive them asynchronously, and more.

**Notice:** This library is still very early in development, and certain features are not fully tested. If you encounter any issues, please submit an issue or PR on [github](https://github.com/Jakobeha/winit-modular).

## The problem

[`winit`](https://crates.io/winit) is the de-facto way to create windows and listen to system events in Rust. However the core struct [`EventLoop`](https://docs.rs/winit/latest/winit/event_loop/struct.EventLoop.html), which allows you to create windows and listen to events, has some annoying restrictions:

- It must be created on the main thread
- It must be created only once
- When `EventLoop::run` ends, your application exits.

which imply more restrictions:

- You can't run multiple event loops simultanously
- You can't stop an event loop and later run another one
- You can't have multiple dependencies which create event loops.

There is also the issue of [inversion of control](https://crates.io/crates/winit-main) which [winit-main](https://crates.io/crates/winit-main) explains and attempts to solve.

## The solution

This crate fixes `winit`'s lack of modularity, [*sort of*](#drawbacks). It provides `EventLoop`s which have a similar api to `winit::event_loop::EventLoop`, but:

- Can exist and run simultaneously on separate threads or even the same thread (see [`run`](https://docs.rs/winit-modular/latest/winit_modular/struct.EventLoop.html#method.run))
- Can run asynchronously (see [`run_async`](https://docs.rs/winit-modular/latest/winit_modular/struct.EventLoop.html#method.run_async))
- Can be polled (see [`run_immediate`](https://docs.rs/winit-modular/latest/winit_modular/struct.EventLoop.html#method.run_immediate)), fixing the "inversion of control" issue
- You can stop calls to any of these and drop the event loops without exiting your entire application.

This works as these `EventLoop`s are actually proxies, which forward their calls and recieve events from the main event loop using asynchronous channels and atomics.

## Drawbacks

This doesn't completely alleviate `winit`'s issues and is not always drop-in replacement.

Most importantly, *you must call `winit_modular::run` exactly once in your application, on the main thread, before using the event loops in this crate*. If you don't you will get a panic message explaining this. as `winit_modular::run` hijacks the main thread, it provides a callback to run the rest of your code on the background thread.

Also the performance penalty from using multiple threads and sending messages across channels. The proxy event loops must communicate with the actual winit event loop across thread bounds, for every operation or intercepted event. That means, often once every frame refresh. Fortunately, modern hardware is generally fast enough and threading is good enough that even then it's a minor performance penalty. But on embedded systems, or if you are spawning a lot of proxy event loops simultaneously, it could be an issue.

## Example (originally from [winit-main](https://crates.io/crates/winit-main))

Without `winit_modular`:

```rust
use winit::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::WindowBuilder,
};

fn main() {
    let event_loop = EventLoop::new();
    let window = WindowBuilder::new().build(&event_loop).unwrap();

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        if matches!(
            event,
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                window_id,
            } if window_id == window.id()
        ) {
            *control_flow = ControlFlow::Exit;
        }
    });
}
```

With `winit-modular`:

```rust
use winit_modular::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop}
};
use pollster::block_on;

fn main() {
    winit_modular::run(|| block_on(async {
        let event_loop = EventLoop::new().await;
        
        let window = event_loop
            .create_window(|builder| builder)
            .await
            .unwrap();

        event_loop.run_async(|event, control_flow| {
            if matches!(
                event,
                Event::WindowEvent {
                    event: WindowEvent::CloseRequested,
                    window_id,
                } if window_id == window.id()
            ) {
                *control_flow = ControlFlow::ExitApp;
            }
        }).await;
    }));
}
```