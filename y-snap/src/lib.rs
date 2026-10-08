//! Library surface of y-snap: just enough for a settings editor to read and
//! write `y-snap.xml` without linking the message loop, hotkey registration
//! or hooks that make up the tray application itself.
//!
//! `main.rs` uses these same modules (via `use y_snap::...`) rather than
//! redeclaring them, so the binary and a settings app like `y-snap-config`
//! always agree on one `Settings` type.

pub mod action;
pub mod config;
pub mod ipc;
