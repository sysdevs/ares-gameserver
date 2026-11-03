//! Compiled scripts, their lookups and the VM.

pub mod iterators;
pub mod ops;
pub mod provider;
pub mod runner;
pub mod state;

/// The active-entity pointers a script may hold (one bit each in `ScriptState`).
pub use ares_shared::pointer::PointerType;
