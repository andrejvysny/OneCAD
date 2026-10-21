//! iPadOS host bridge. Contract: include/onecad_ios.h and docs/ios/ARCHITECTURE.md;
//! frame loop, threading and input mapping: docs/ios/HOST.md.
//!
//! Runs the shared `onecad-ui` editor (desktop feature off) without eframe: the
//! UIKit host owns the `CAMetalLayer`, events and accessibility elements and
//! calls these C functions on its main thread.
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]
mod a11y;
mod abi;
mod abi_input;
mod app;
mod ffi;
mod frame;
mod gpu;
mod host;
mod input;
mod keys;
mod lifecycle;
#[cfg(feature = "test-hooks")]
mod test_hooks;
mod wake;
