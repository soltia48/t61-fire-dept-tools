//! Library for ARIB STD-T61 SCPC/FDMA Fire-Department channel decoding.
//!
//! Sync-word and frame-format definitions are taken from
//! ARIB STD-T61 v1.2 part 2 (FDMA part).
//!
//! The public entry point is [`Decoder`], which consumes 48-byte FDMA
//! frames (the output of `t61_frame_slicer`) and emits one JSONL record
//! per frame to its writer.

pub mod convo;
pub mod gps;
pub mod json;
pub mod primitives;
pub mod slicer;
pub mod state;
pub mod tables;

mod decoder;

pub use decoder::{Decoder, OutputMode};
pub use slicer::Slicer;
pub use state::{DecoderState, MField, PscState};
