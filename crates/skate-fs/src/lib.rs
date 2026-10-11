//! File access for the engine. Natively (and on WASI) these are `std::fs` itself, so behaviour is
//! unchanged.
//! A browser tab has no file system: there the same calls read the player's own converted
//! installation, which the loader page reads from the folder they chose, and writes (settings)
//! go to memory for the session (docs/web-port/installation.md).
//!
//! Use these instead of `std::fs` and the file-system methods on `Path` (`is_file`, `is_dir`,
//! `exists`, `canonicalize`) in any code the browser build runs.
#![forbid(unsafe_code)]

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
mod native;
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub use native::*;

#[cfg(any(all(target_arch = "wasm32", target_os = "unknown"), test))]
mod memory;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod web;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub use web::*;
