//! ARIB STD-T61 SCPC/FDMA frame extractor.
//!
//!     usage: t61_frame_slicer < input.bin > output.t61
//!
//! Reads a 2-bit-per-symbol stream on stdin (each input byte holds
//! four symbols) and writes 48-byte FDMA frames on stdout.
//!
//! Both descriptors are accessed as raw file descriptors so reads and
//! writes map directly to `read(2)` / `write(2)` syscalls — no
//! Rust-side buffering on either side, so each frame reaches
//! downstream consumers as soon as it is produced (real-time pipe
//! safe).

use std::io::{self, Write};
use std::mem::ManuallyDrop;
use std::os::fd::FromRawFd;

use t61_fd::Slicer;

fn main() -> io::Result<()> {
    let stdin_file = ManuallyDrop::new(unsafe { std::fs::File::from_raw_fd(0) });
    let input: &std::fs::File = &stdin_file;
    let stdout_file = ManuallyDrop::new(unsafe { std::fs::File::from_raw_fd(1) });
    let mut output: &std::fs::File = &stdout_file;

    for frame in Slicer::new(input) {
        output.write_all(&frame?)?;
    }
    Ok(())
}
