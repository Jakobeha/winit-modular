# winit-modular: use `winit` without relying on a single `EventLoop` on the main thread


[`winit`](https://crates.io/winit) is the de-facto way to create windows and listen to system events in Rust. However the core struct [`EventLoop`](https://docs.rs/winit/latest/winit/event_loop/struct.EventLoop.html), which allows you to create windows and listen to events, has some annoying restrictions:

- It must be created on the main thread
- It must be created only once
- When `EventLoop::run` ends, your application exits.

which imply

- You can't run multiple event loops simultanously
- You can't stop an event loop and later run another one
- You can't have multiple dependencies which create event loops.

This crate fixes these issues, sort of, by providing `winit_modular::run` and `EventLoopProxy`. `winit_modular::run` must be called once on the main thread, and it blocks the main thread, as it creates and runs the event loop. But allows you to use `EventLoopProxy`s. The API of `EventLoopProxy` is very similar to `EventLoop`, but provides these advantages:

- You can create multiples of these and even run them at the same time, on separate threads
- You can create these on separate threads
- You can stop these loops without exiting your entire application

Note that there is a performance penalty, as `EventLoopProxy`s must communicate with the `EventLoop` across thread bounds, for every call or intercepted event. Modern hardware is generally fast enough that this should not be an issue, but this could be an issue on embedded hardware or if you are sending a particularly large number of calls or intercepting a particularly large number of events (i.e. more than `Render` events which are sent once per frame).