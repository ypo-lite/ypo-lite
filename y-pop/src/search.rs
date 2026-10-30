//! Search: finding a menu item, a group or a command by typing a few
//! letters. `docs/overlay-modes.md` §2 is the design, `docs/search-plan.md`
//! the build order.
//!
//! Everything here is pure Rust with no Win32 and no window — the matcher
//! and the hotkey-combination type take no `y-pop` type at all, so they can
//! move to `y-core` when `y-term` needs them (spec §2.12).
//!
//! * [`matcher`]: fuzzy subsequence scoring of one query word against one
//!   piece of text.
//! * [`actions`]: which actions apply to which result.
//! * [`columns`]: the Miller columns the tray menus section opens as.
//! * [`commands`]: the `>` commands, the `?` prefixes and the rows they make.
//! * [`keycheck`]: is a key combination free? The table of the keys Windows keeps.
//! * [`display`]: the text of a result row (matched runs, headings, second line).
//! * [`files`]: the files of the `/` prefix, listed on a thread of their own.
//! * [`history`]: what Windows remembers, for Recent and a ranking bonus.
//! * [`hotkey`]: a key combination as a set, for queries like `shift+ctrl+esc`.
//! * [`index`]: the flat list of everything searchable, built from a
//!   [`crate::model::ConfigTree`] at load and reload, never per keystroke.
//! * [`input`]: the text box's state machine (text, caret, selection, keys).
//! * [`keys`]: keys and shortcuts (`Ctrl+Shift+Enter`), parsed and checked.
//! * [`layout`]: where the window's parts go (pure geometry, hit-testing).
//! * [`query`]: running a typed query over the index.
//! * [`state`]: the box's behaviour — events in, outcomes out.
//! * [`results`]: the Top hit / groups layout of a result list.
//! * [`sections`]: the sections the empty box lists, and the places a box can be scoped to.

pub mod actions;
pub mod columns;
pub mod commands;
pub mod display;
pub mod files;
pub mod history;
pub mod hotkey;
pub mod index;
pub mod input;
pub mod keycheck;
pub mod keys;
pub mod layout;
pub mod matcher;
pub mod query;
pub mod results;
pub mod sections;
pub mod state;
