//! Library surface of y-coustic: the parts a settings editor shares with the
//! tray application (the pure model and the configuration), without the
//! message loop, hooks or COM.

pub mod appearance;
pub mod config;
pub mod controls;
pub mod edit;
pub mod input;
pub mod ipc;
pub mod kept;
pub mod look;
pub mod model;
pub mod paint;
