//! ARIB STD-T61 SCPC/FDMA frame slicer.
//!
//! Reads a 2-bit-per-symbol stream and yields aligned 48-byte FDMA
//! frames.  Frames whose sync word cannot be located are replaced
//! with all-zero placeholder frames (emitted downstream as
//! `"type": "no_signal"`) so consumers preserve frame numbering.
//!
//! ## Read-granularity contract
//!
//! The slicer's algorithm is sensitive to read granularity: short
//! reads from the underlying source produce different (but still
//! valid) framing decisions than full reads.  For byte-identical
//! output to the C reference on regular-file inputs, supply a
//! `Read` whose `read()` returns full chunks (e.g. a raw file
//! descriptor wrapper rather than `io::stdin()`'s buffered
//! `StdinLock`).

use std::io::{self, Read};

/// Frame layout, in 2-bit symbols.
const FRAME_SYMBOLS: usize = 192; // 384 bits / 2
/// Frame size in packed bytes (4 symbols / byte).
pub const FRAME_BYTES: usize = FRAME_SYMBOLS / 4;
/// Sync-word position within a frame, in symbols.
const SYNC_WORD_OFFSET: usize = 92;
/// Length (in symbols) of the longest sync word (SS1).
const SYNC_WINDOW_SYMBOLS: usize = 16;
/// Maximum drift of the sync-word position, in symbols.
const LP_R_FLUCT: usize = 8 / 2;
/// Per-sync-word symbol-error budget when matching.
const ERROR_MAX: u32 = 1;
/// Length (in symbols) of the smallest sync word (S2 / S6).
const SMALLEST_SYNC_LEN: usize = 10;

const NO_SIGNAL_FRAME: [u8; FRAME_BYTES] = [0; FRAME_BYTES];

/// Sync words from ARIB STD-T61 v1.2 part 2 (FDMA part), expressed as
/// 2-bit symbols (MSB pair first).  Used to patch the input buffer
/// with the canonical pattern after a tolerated match.
const SW_SS1: &[u8] = &[0, 2, 3, 3, 2, 1, 1, 0, 3, 1, 0, 0, 1, 2, 2, 3];
const SW_S2: &[u8] = &[2, 1, 3, 1, 0, 2, 0, 3, 1, 2];
const SW_S6: &[u8] = &[0, 1, 3, 2, 1, 1, 1, 2, 3, 3];

/// Packed sync-word descriptor.  The 32-bit pattern is top-aligned so
/// the same sliding window matches sync words of different lengths
/// via the per-entry mask.
struct PackedSw {
    pattern: u32,
    mask: u32,
    symbols: &'static [u8],
}

const PACKED_SW_TAB: &[PackedSw] = &[
    PackedSw {
        pattern: 0x1e56f000,
        mask: 0xfffff000,
        symbols: SW_S6,
    },
    PackedSw {
        pattern: 0x2f94d06b,
        mask: 0xffffffff,
        symbols: SW_SS1,
    },
    PackedSw {
        pattern: 0x9d236000,
        mask: 0xfffff000,
        symbols: SW_S2,
    },
];

// -------- bit-level pack / unpack ---------------------------------

#[inline]
fn unpack_byte(b: u8) -> [u8; 4] {
    [(b >> 6) & 3, (b >> 4) & 3, (b >> 2) & 3, b & 3]
}

#[inline]
fn pack_4(s: &[u8]) -> u8 {
    ((s[0] & 3) << 6) | ((s[1] & 3) << 4) | ((s[2] & 3) << 2) | (s[3] & 3)
}

fn pack_frame(symbols: &[u8]) -> [u8; FRAME_BYTES] {
    let mut out = [0u8; FRAME_BYTES];
    for (i, chunk) in symbols.chunks_exact(4).take(FRAME_BYTES).enumerate() {
        out[i] = pack_4(chunk);
    }
    out
}

// -------- sync-word matching --------------------------------------

/// Compare `window` against every known sync word.  On a match within
/// `ERROR_MAX` symbol errors, patch `p` with the canonical pattern
/// and return true.
fn match_window(window: u32, p: &mut [u8]) -> bool {
    for w in PACKED_SW_TAB {
        let diff = (window ^ w.pattern) & w.mask;
        if diff.count_ones() <= ERROR_MAX {
            p[..w.symbols.len()].copy_from_slice(w.symbols);
            return true;
        }
    }
    false
}

/// Search for a sync word inside `buf`.  Tests symbol offsets starting
/// from `SYNC_WORD_OFFSET`; the most likely position (`LP_R_FLUCT`)
/// is probed first as a fast path.
fn find_sync(buf: &mut [u8], valid: usize) -> Option<usize> {
    if valid < FRAME_SYMBOLS {
        return None;
    }
    let positions = valid - FRAME_SYMBOLS + 1;
    let base = SYNC_WORD_OFFSET;

    let mut window = buf[base..base + SYNC_WINDOW_SYMBOLS]
        .iter()
        .fold(0u32, |acc, &b| (acc << 2) | (b & 3) as u32);

    if LP_R_FLUCT > 0 {
        let w = buf[base + SYNC_WINDOW_SYMBOLS..base + SYNC_WINDOW_SYMBOLS + LP_R_FLUCT]
            .iter()
            .fold(window, |acc, &b| (acc << 2) | (b & 3) as u32);
        if match_window(w, &mut buf[base + LP_R_FLUCT..]) {
            return Some(LP_R_FLUCT);
        }
    }

    for i in 0..positions {
        if !(LP_R_FLUCT > 0 && i == LP_R_FLUCT) && match_window(window, &mut buf[base + i..]) {
            return Some(i);
        }
        window = (window << 2) | (buf[base + i + SYNC_WINDOW_SYMBOLS] & 3) as u32;
    }
    None
}

// ============================================================ Slicer

/// Streaming frame slicer.  Wraps a [`Read`] and yields aligned
/// 48-byte frames via [`Slicer::next_frame`] or by iteration.
pub struct Slicer<R: Read> {
    input: R,
    rbuf: Vec<u8>,
    valid: usize,
    /// One-slot buffer for a real frame held back behind a no_signal frame.
    pending: Option<[u8; FRAME_BYTES]>,
    eof: bool,
    done: bool,
}

impl<R: Read> Slicer<R> {
    pub fn new(input: R) -> Self {
        Slicer {
            input,
            rbuf: vec![0; FRAME_SYMBOLS * 3], // sliding window: 3 frames
            valid: 0,
            pending: None,
            eof: false,
            done: false,
        }
    }

    /// Pull the next frame.  Returns `Ok(None)` at end of stream.
    pub fn next_frame(&mut self) -> io::Result<Option<[u8; FRAME_BYTES]>> {
        if let Some(p) = self.pending.take() {
            return Ok(Some(p));
        }
        if self.done {
            return Ok(None);
        }

        loop {
            if !self.eof {
                if self.valid <= FRAME_SYMBOLS * 2 {
                    if self.read_more()? == 0 {
                        self.eof = true;
                    }
                    continue;
                }
                match find_sync(&mut self.rbuf, self.valid) {
                    None => self.slide_forward(),
                    Some(ret) => return Ok(Some(self.consume_with_no_signal(ret))),
                }
            } else {
                // Drain phase: process whatever sync words remain, then
                // emit one trailing no_signal frame if there's enough leftover.
                if self.valid >= FRAME_SYMBOLS {
                    if let Some(ret) = find_sync(&mut self.rbuf, self.valid) {
                        return Ok(Some(self.consume(ret)));
                    }
                }
                self.done = true;
                if self.valid >= SMALLEST_SYNC_LEN {
                    return Ok(Some(NO_SIGNAL_FRAME));
                }
                return Ok(None);
            }
        }
    }

    /// Read up to one frame's worth of packed bytes into `rbuf`,
    /// expanding to one symbol per byte.  Returns the number of input
    /// bytes consumed (0 at EOF).
    fn read_more(&mut self) -> io::Result<usize> {
        let mut inbuf = [0u8; FRAME_BYTES];
        let n = self.input.read(&mut inbuf)?;
        let dest = &mut self.rbuf[self.valid..self.valid + n * 4];
        for (i, &b) in inbuf[..n].iter().enumerate() {
            dest[i * 4..i * 4 + 4].copy_from_slice(&unpack_byte(b));
        }
        self.valid += n * 4;
        Ok(n)
    }

    /// Slide the buffer forward by `FRAME_SYMBOLS - LP_R_FLUCT`,
    /// keeping the trailing tail as the new head (so the next sync
    /// search can absorb up to `LP_R_FLUCT` symbols of drift).
    fn slide_forward(&mut self) {
        let drop_amt = FRAME_SYMBOLS - LP_R_FLUCT;
        self.valid -= drop_amt;
        self.rbuf.copy_within(drop_amt..drop_amt + self.valid, 0);
    }

    /// Consume one frame at `ret`, sliding the buffer.  Returns the
    /// packed 48-byte output.
    fn consume(&mut self, ret: usize) -> [u8; FRAME_BYTES] {
        let frame = pack_frame(&self.rbuf[ret..ret + FRAME_SYMBOLS]);
        let drop_amt = ret + FRAME_SYMBOLS - LP_R_FLUCT;
        self.valid -= drop_amt;
        self.rbuf.copy_within(drop_amt..drop_amt + self.valid, 0);
        frame
    }

    /// Consume `ret`, but if it's larger than the expected drift,
    /// stash the consumed frame and emit a no_signal frame first.
    fn consume_with_no_signal(&mut self, ret: usize) -> [u8; FRAME_BYTES] {
        let frame = self.consume(ret);
        if ret > FRAME_SYMBOLS - LP_R_FLUCT {
            self.pending = Some(frame);
            NO_SIGNAL_FRAME
        } else {
            frame
        }
    }
}

impl<R: Read> Iterator for Slicer<R> {
    type Item = io::Result<[u8; FRAME_BYTES]>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_frame().transpose()
    }
}

// ============================================================ tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_yields_no_frames() {
        let input: &[u8] = &[];
        let mut slicer = Slicer::new(input);
        assert!(slicer.next_frame().unwrap().is_none());
    }

    #[test]
    fn under_one_frame_yields_no_real_frame() {
        // 47 bytes < FRAME_BYTES (48), so no sync window can be tested.
        let input = vec![0u8; 47];
        let mut slicer = Slicer::new(input.as_slice());
        // No real frame; possibly a trailing no_signal frame if leftover >= 10.
        let first = slicer.next_frame().unwrap();
        // 47 bytes = 188 symbols which is plenty for trailing no_signal frame.
        assert_eq!(first, Some(NO_SIGNAL_FRAME));
        assert!(slicer.next_frame().unwrap().is_none());
    }

    #[test]
    fn iter_is_fused_at_done() {
        let input: &[u8] = &[];
        let mut slicer = Slicer::new(input);
        assert!(slicer.next().is_none());
        assert!(slicer.next().is_none());
    }
}
