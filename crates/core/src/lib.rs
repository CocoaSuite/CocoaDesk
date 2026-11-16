//! Core logic for CocoaDesk: data model, calendar rules, storage and imports.
//!
//! Everything here is deterministic and runs locally. The desktop shell
//! (`src-tauri`) only wires these pieces to the window, tray and timer.

pub mod boards;
pub mod calendar;
pub mod error;
pub mod import;
pub mod model;
pub mod news;
pub mod service;
pub mod store;
pub mod tasks;

pub use error::{Error, Result};
