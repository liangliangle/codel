//! Shared utilities used by both `codel-shell` and its downstream clients
//! (e.g. `codel-pager-render`). This crate sits upstream of `codel-shell`
//! so it must never depend on it.

pub mod clipboard;
pub mod placeholder_images;
pub mod session;
pub mod stderr;
pub mod ui_config;
