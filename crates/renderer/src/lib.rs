//! In-process diagram engine: a worker thread that lays diagrams out and caches the results,
//! plus export of finished diagrams to SVG and PNG.
mod engine;
pub mod export;

pub use engine::Engine;
