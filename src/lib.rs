//! Agent Graph records how AI coding sessions relate to each other, from
//! provider hooks, as an append-only event log. See `docs/design.md`.
//!
//! Without the default `cli` feature the crate is just the event model, the
//! reducer and the timeline logic, which also build for WebAssembly.

pub mod adapter;
pub mod clock;
pub mod event;
pub mod paths;
pub mod reducer;
pub mod render;
pub mod store;
pub mod timeline;

#[cfg(feature = "cli")]
pub mod cli;
#[cfg(feature = "cli")]
pub mod emit;
#[cfg(feature = "cli")]
pub mod image;
#[cfg(feature = "cli")]
pub mod install;
#[cfg(feature = "cli")]
pub mod live;
#[cfg(feature = "cli")]
pub mod remote;
#[cfg(feature = "cli")]
pub mod view;
