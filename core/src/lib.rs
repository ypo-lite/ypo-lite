//! Shared Win32 plumbing for the Ypo-Lite applications.
//!
//! Everything here comes from the `skeleton/` reference application, made
//! reusable: the `unsafe` and the Win32 idioms live in this crate, the
//! applications stay plain Rust with a `match` on messages.
//!
//! # Shape of an application
//!
//! ```no_run
//! use y_core::prelude::*;
//!
//! const APP: AppInfo = AppInfo::new("Y-Snap", "YpoLite.YSnap").single_instance();
//!
//! struct App;
//!
//! impl WindowHandler for App {
//!     fn handle_message(
//!         &mut self,
//!         hwnd: HWND,
//!         msg: u32,
//!         wparam: WPARAM,
//!         lparam: LPARAM,
//!     ) -> Option<LRESULT> {
//!         let _ = (hwnd, msg, wparam, lparam);
//!         None
//!     }
//! }
//!
//! fn main() -> ! {
//!     y_core::app::start(&APP, || Box::new(App))
//! }
//! ```
//!
//! [`app::start`] owns the boring half of every application: the single
//! instance guard, the hidden message window, the start-up error box, the
//! working-set trim and the message loop. What an application adds on top is
//! picked à la carte:
//!
//! | Module | For |
//! | ------ | --- |
//! | [`tray`] | notification area icon, balloon tips |
//! | [`menu`] | popup menus, checkable and disabled items |
//! | [`hotkey`] | system-wide hotkeys, with a low-level-hook override |
//! | [`overlay`] | layered, click-through windows drawn from a pixel buffer |
//! | [`monitor`] | monitor geometry, work areas, per-monitor DPI |
//! | [`config`] | portable `.ini` settings next to the executable |
//! | [`xml`] | a small XML reader, for settings files that are not `.ini` |
//! | [`theme`] | whether Windows is asking for a dark palette |
//! | [`startup`] | run-at-logon registration |
//! | [`resources`] | icons embedded by `build.rs` |
//! | [`window`] | window classes and the message loop, for anything else |
//!
//! # Features
//!
//! * `win` (default) — the modules above.
//! * `build` — [`build`], the `build.rs` helper. Enable it *without*
//!   `default-features` in `[build-dependencies]`, so compiling the build
//!   script does not also compile the `windows` crate.

#![cfg_attr(docsrs, feature(doc_cfg))]

#[cfg(feature = "build")]
pub mod build;

#[cfg(feature = "win")]
pub mod app;
#[cfg(feature = "win")]
pub mod config;
#[cfg(feature = "win")]
pub mod hotkey;
#[cfg(feature = "win")]
pub mod ids;
#[cfg(feature = "win")]
pub mod memory;
#[cfg(feature = "win")]
pub mod menu;
#[cfg(feature = "win")]
pub mod monitor;
#[cfg(feature = "win")]
pub mod overlay;
#[cfg(feature = "win")]
pub mod prelude;
#[cfg(feature = "win")]
pub mod resources;
#[cfg(feature = "win")]
pub mod single_instance;
#[cfg(feature = "win")]
pub mod startup;
#[cfg(feature = "win")]
pub mod theme;
#[cfg(feature = "win")]
pub mod tray;
#[cfg(feature = "win")]
pub mod virtual_desktop;
#[cfg(feature = "win")]
pub mod window;

// Pure Rust, no Win32: useful to a build script and to a test on any host.
pub mod xml;

#[cfg(feature = "win")]
pub use windows::core::{Error, Result};
