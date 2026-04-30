//! ARIB STD-T61 SCPC/FDMA Fire-Department channel decoder.
//!
//!     usage: t61_fd_decoder [-c|--celp] < output.t61
//!
//! Reads 48-byte FDMA frames on stdin and emits JSONL records (or
//! CELP-only hex lines with `-c`) to stdout.
//!
//! Both descriptors are accessed as raw file descriptors so reads and
//! writes map directly to `read(2)` / `write(2)` syscalls — no
//! Rust-side buffering on either side, so each frame's output reaches
//! downstream consumers as soon as it is produced (real-time pipe
//! safe).

use std::io::{self, Read};
use std::mem::ManuallyDrop;
use std::os::fd::FromRawFd;

use t61_fd::{Decoder, OutputMode};

/// Fill `buf` with as many bytes as possible.  Returns the number of
/// bytes filled (equals `buf.len()` for full reads, 0 at EOF).
fn read_full<R: Read>(input: &mut R, buf: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match input.read(&mut buf[filled..])? {
            0 => break,
            n => filled += n,
        }
    }
    Ok(filled)
}

fn parse_args() -> OutputMode {
    let mut mode = OutputMode::Json;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "-c" | "--celp" => mode = OutputMode::CelpOnly,
            "-h" | "--help" => {
                eprintln!("usage: t61_fd_decoder [-c|--celp] < output.t61");
                std::process::exit(0);
            }
            other => {
                eprintln!("t61_fd_decoder: unknown argument: {}", other);
                eprintln!("usage: t61_fd_decoder [-c|--celp] < output.t61");
                std::process::exit(2);
            }
        }
    }
    mode
}

fn main() -> io::Result<()> {
    let mode = parse_args();
    let stdin_file = ManuallyDrop::new(unsafe { std::fs::File::from_raw_fd(0) });
    let mut input: &std::fs::File = &stdin_file;
    let stdout_file = ManuallyDrop::new(unsafe { std::fs::File::from_raw_fd(1) });
    let output: &std::fs::File = &stdout_file;
    let mut decoder = Decoder::with_mode(output, mode);

    let mut frame = [0u8; 48];
    let mut frame_num: u64 = 0;
    loop {
        let n = read_full(&mut input, &mut frame)?;
        if n < 48 {
            break;
        }
        decoder.process_frame(&frame, frame_num)?;
        frame_num += 1;
    }
    Ok(())
}
