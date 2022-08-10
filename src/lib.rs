#![doc = include_str!("../README.md")]
#![feature(drain_filter)]
#![feature(box_into_inner)]

/// Proxy event loops and control flow type.
pub mod event_loop;
/// Events received by the proxy event loops.
pub mod event;
/// Futures, since most of the operations are across threads.
#[doc(hidden)]
pub mod future;
/// Messages sent between the proxy event loops and shared event loop.
mod messages;
/// Function to initialize the main event loop for the proxies.
mod run;

pub use run::*;