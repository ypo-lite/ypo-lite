//! Y-Pop — application launcher, driven by hotkeys and popup menus.
//!
//! The Rust rewrite of PortAL. See `docs/y-pop-plan.md` for the design and
//! its phased roadmap; this crate currently implements phase 1 of that
//! roadmap only — the configuration data model and its XML parser, both
//! pure Rust with no Win32 involved, so they build and test on any host.
//! The Win32 shell (tray icon, hotkey registration, the popup itself) is a
//! later phase, in the `y-pop` binary (`src/main.rs`).
//!
//! A library target, like `y-core`, rather than modules private to the
//! binary: `model`/`config`/`variables` are meant to be reused unchanged by
//! whichever renderer phase comes next, and keeping them in a `lib.rs`
//! means the compiler's dead-code analysis judges them by whether anything
//! in the crate (tests included) uses them, not by whether `main` happens
//! to reach every one of them yet.

pub mod builtins;
pub mod config;
pub mod ipc;
pub mod launch;
pub mod model;
pub mod settings;
pub mod skins;
pub mod theme;
pub mod variables;
